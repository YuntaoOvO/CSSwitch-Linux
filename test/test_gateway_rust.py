import http.client
import json
import os
import pathlib
import socket
import subprocess
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_GATEWAY_BIN = ROOT / "desktop" / "gateway" / "target" / "debug" / "csswitch-gateway"
STAGED_GATEWAY_DIR = ROOT / "desktop" / "src-tauri" / "binaries"


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    if port == 8765:
        return free_port()
    return port


def gateway_bin():
    raw = os.environ.get("CSSWITCH_GATEWAY_BIN")
    if raw and pathlib.Path(raw).is_file():
        return pathlib.Path(raw)
    if DEFAULT_GATEWAY_BIN.is_file():
        return DEFAULT_GATEWAY_BIN
    for path in sorted(STAGED_GATEWAY_DIR.glob("csswitch-gateway-*")):
        if path.is_file():
            return path
    return None


def recv_http_head(sock):
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            break
        data += chunk
    return data


class MockUpstream(ThreadingHTTPServer):
    allow_reuse_address = True

    def __init__(self, response_body, content_type="application/json"):
        self.requests = []
        self.response_body = response_body
        self.content_type = content_type
        super().__init__(("127.0.0.1", free_port()), MockHandler)


class MockHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        self.server.requests.append(
            {
                "path": self.path,
                "headers": {k.lower(): v for k, v in self.headers.items()},
                "body": body,
            }
        )
        payload = self.server.response_body
        self.send_response(200)
        self.send_header("content-type", self.server.content_type)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_args):
        pass


class EchoServer:
    def __init__(self):
        self.port = free_port()
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()
        self.ready.wait(2)

    def _serve(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as srv:
            srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            srv.bind(("127.0.0.1", self.port))
            srv.listen(1)
            self.ready.set()
            conn, _ = srv.accept()
            with conn:
                data = conn.recv(4096)
                conn.sendall(data)


class RustGatewayLoopback(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bin = gateway_bin()
        if cls.bin is None:
            raise unittest.SkipTest("csswitch-gateway binary not built")

    def start_gateway(self, upstream_url=None, secret="secret"):
        port = free_port()
        env = os.environ.copy()
        env.update(
            {
                "DEEPSEEK_API_KEY": "fake-deepseek-key",
                "CSSWITCH_AUTH_TOKEN": secret,
                "CSSWITCH_TOOLUSE_SHIM": "off",
            }
        )
        if upstream_url:
            env["CSSWITCH_UPSTREAM_URL"] = upstream_url
        proc = subprocess.Popen(
            [
                str(self.bin),
                "--provider",
                "deepseek",
                "--port",
                str(port),
                "--auth-token",
                "cli-secret-should-lose",
            ],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        deadline = time.time() + 5
        while time.time() < deadline:
            try:
                conn = http.client.HTTPConnection("127.0.0.1", port, timeout=0.2)
                conn.request("GET", f"/{secret}/health")
                resp = conn.getresponse()
                resp.read()
                conn.close()
                if resp.status == 200:
                    return proc, port
            except OSError:
                time.sleep(0.05)
        proc.terminate()
        stderr = ""
        try:
            _, stderr = proc.communicate(timeout=1)
        except Exception:
            proc.kill()
        raise RuntimeError(f"gateway did not become healthy: {stderr}")

    def stop_gateway(self, proc):
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)
        for handle in (proc.stdout, proc.stderr):
            if handle:
                handle.close()

    def test_auth_and_models(self):
        proc, port = self.start_gateway()
        try:
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            conn.request("GET", "/v1/models")
            forbidden = conn.getresponse()
            self.assertEqual(forbidden.status, 403)
            forbidden_body = json.loads(forbidden.read())
            conn.close()
            self.assertEqual(forbidden_body["type"], "error")
            self.assertEqual(forbidden_body["error"]["type"], "permission_error")

            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            conn.request("GET", "/secret/v1/models")
            resp = conn.getresponse()
            body = json.loads(resp.read())
            conn.close()
            self.assertEqual(resp.status, 200)
            self.assertEqual(body["first_id"], "claude-opus-4-8")
            self.assertEqual(body["last_id"], "claude-haiku-4-5")

            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            conn.request("GET", "/secret/health")
            resp = conn.getresponse()
            body = json.loads(resp.read())
            conn.close()
            self.assertEqual(resp.status, 200)
            self.assertEqual(body, {"status": "ok", "provider": "deepseek"})
        finally:
            self.stop_gateway(proc)

    def test_nonstream_maps_request_and_preserves_content_length(self):
        upstream = MockUpstream(b'{"id":"msg_mock","type":"message"}')
        thread = threading.Thread(target=upstream.serve_forever, daemon=True)
        thread.start()
        proc, port = self.start_gateway(
            upstream_url=f"http://127.0.0.1:{upstream.server_port}/anthropic/v1/messages"
        )
        try:
            request = {
                "model": "claude-opus-4-8",
                "max_tokens": 100000,
                "thinking": {"type": "auto"},
                "messages": [{"role": "user", "content": "hi"}],
            }
            raw = json.dumps(request).encode()
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            conn.request(
                "POST",
                "/secret/v1/messages",
                body=raw,
                headers={"content-type": "application/json"},
            )
            resp = conn.getresponse()
            body = resp.read()
            conn.close()
            self.assertEqual(resp.status, 200)
            self.assertEqual(resp.getheader("content-length"), str(len(body)))
            self.assertEqual(body, b'{"id":"msg_mock","type":"message"}')

            self.assertEqual(len(upstream.requests), 1)
            req = upstream.requests[0]
            self.assertEqual(req["path"], "/anthropic/v1/messages")
            self.assertEqual(req["headers"]["x-api-key"], "fake-deepseek-key")
            self.assertEqual(req["headers"]["anthropic-version"], "2023-06-01")
            mapped = json.loads(req["body"])
            self.assertEqual(mapped["model"], "deepseek-v4-pro")
            self.assertEqual(mapped["max_tokens"], 65536)
            self.assertEqual(mapped["thinking"]["type"], "adaptive")
        finally:
            self.stop_gateway(proc)
            upstream.shutdown()
            upstream.server_close()

    def test_stream_passthrough_dechunks_same_payload(self):
        payload = b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n"
        upstream = MockUpstream(payload, content_type="text/event-stream")
        thread = threading.Thread(target=upstream.serve_forever, daemon=True)
        thread.start()
        proc, port = self.start_gateway(
            upstream_url=f"http://127.0.0.1:{upstream.server_port}/anthropic/v1/messages"
        )
        try:
            request = {
                "model": "claude-haiku-4-5",
                "stream": True,
                "messages": [{"role": "user", "content": "hi"}],
            }
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            conn.request(
                "POST",
                "/secret/v1/messages",
                body=json.dumps(request).encode(),
                headers={"content-type": "application/json"},
            )
            resp = conn.getresponse()
            body = resp.read()
            conn.close()
            self.assertEqual(resp.status, 200)
            self.assertEqual(resp.getheader("transfer-encoding"), "chunked")
            self.assertEqual(body, payload)
        finally:
            self.stop_gateway(proc)
            upstream.shutdown()
            upstream.server_close()

    def test_connect_blocks_claude_hosts_and_tunnels_other_hosts(self):
        proc, port = self.start_gateway()
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as sock:
                sock.sendall(b"CONNECT claude.ai:443 HTTP/1.1\r\nhost: claude.ai:443\r\n\r\n")
                head = recv_http_head(sock)
            self.assertIn(b"401", head.split(b"\r\n", 1)[0])

            echo = EchoServer()
            with socket.create_connection(("127.0.0.1", port), timeout=2) as sock:
                target = f"CONNECT 127.0.0.1:{echo.port} HTTP/1.1\r\nhost: 127.0.0.1:{echo.port}\r\n\r\n"
                sock.sendall(target.encode())
                data = recv_http_head(sock)
                self.assertIn(b"200", data.split(b"\r\n", 1)[0])
                sock.sendall(b"ping")
                self.assertEqual(sock.recv(4), b"ping")
        finally:
            self.stop_gateway(proc)


if __name__ == "__main__":
    unittest.main()
