# S1b packaged app smoke checkpoint

Date: 2026-07-09
Scope: S1b Rust gateway sidecar packaged app resource smoke.

## Current main state

- Branch: `main`
- Remote alignment before this record: `main...origin/main`
- Latest commit: `a5766636a6c197282577de0bdfc98b39e6a1c8a6`
- Latest merge: `Merge pull request #51 from SuperJJ007/codex/s1b-packaged-sidecar-smoke`
- Prior S1b merge: `ee2986c3c01f2df4ef911ee5ee562aa17ce341b6` (`#50`)

## Build command

The app bundle smoke used:

```bash
PATH="$HOME/.cargo/bin:$PATH" npm run tauri build -- --bundles app
```

Run directory:

```text
/Users/superjj/ccproj/CSswitch/desktop
```

The first attempt without the `$HOME/.cargo/bin` PATH prefix did not enter
compilation; Tauri could not find `cargo` for `cargo metadata`.

## Verified packaged layout

Built app:

```text
/Users/superjj/ccproj/CSswitch/desktop/src-tauri/target/release/bundle/macos/CSSwitch.app
```

Observed files:

```text
Contents/MacOS/desktop
Contents/MacOS/csswitch-gateway
Contents/Resources/proxy/*.py
Contents/Resources/scripts/*.sh
```

Sidecar facts:

- Path: `Contents/MacOS/csswitch-gateway`
- Mode: `-rwxr-xr-x`
- File type: `Mach-O 64-bit executable arm64`
- `codesign --verify --deep --strict --verbose=2` passed for the local `.app`
  bundle.

The important packaging detail is that Tauri `externalBin` placed the sidecar
next to the main app executable under `Contents/MacOS/`, not under
`Contents/Resources/binaries/`.

## Lookup implication

The current runtime lookup covers this real `.app` layout through the
`current_exe().parent()` branch in `gateway_bin_path_from()`.

Concrete smoke check:

```text
current_exe_parent=/Users/superjj/ccproj/CSswitch/desktop/src-tauri/target/release/bundle/macos/CSSwitch.app/Contents/MacOS
found=/Users/superjj/ccproj/CSswitch/desktop/src-tauri/target/release/bundle/macos/CSSwitch.app/Contents/MacOS/csswitch-gateway
sidecar_executable=yes
```

## Claim boundary

This checkpoint verifies only the packaged app file layout and sidecar lookup
precondition.

It does not verify:

- CI green.
- GUI E2E.
- Live provider behavior.
- Real `~/.claude-science`.
- Real token or `.env` contents.
- Port `8765`.
- Apple Developer ID signing.
- Notarization.
- Gatekeeper behavior after download/quarantine.
- Full packaged app runtime flow.

## S1b closeout wording

S1b can be described as:

> The packaged Tauri `.app` built from `main` contains an executable Rust
> gateway sidecar, and the current packaged lookup path can find that sidecar
> from the main executable directory.

It should not be described as Python-free, live-provider verified, GUI E2E
verified, or Gatekeeper verified.
