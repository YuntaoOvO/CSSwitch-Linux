use std::fmt;

pub const POLL_INTERVAL_MS: u64 = 500;
pub const LOCAL_HEALTH_TIMEOUT_MS: u64 = 8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    ActivateProfile,
    UpdateActiveConnection,
    ValidateConnection,
    OneClickLogin,
    StartProxy,
    StopProxy,
    StartScience,
    StopScience,
}

impl fmt::Display for OperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationKind::ActivateProfile => write!(f, "activate_profile"),
            OperationKind::UpdateActiveConnection => write!(f, "update_active_connection"),
            OperationKind::ValidateConnection => write!(f, "validate_connection"),
            OperationKind::OneClickLogin => write!(f, "one_click_login"),
            OperationKind::StartProxy => write!(f, "start_proxy"),
            OperationKind::StopProxy => write!(f, "stop_proxy"),
            OperationKind::StartScience => write!(f, "start_science"),
            OperationKind::StopScience => write!(f, "stop_science"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStage {
    Start,
    ConfigRead,
    ScratchUpstreamProbe,
    ProxyStart,
    ProxyHealth,
    SandboxLogin,
    SandboxStart,
    SandboxHealth,
    SandboxIdentity,
    OpenBrowser,
    Done,
}

impl fmt::Display for OperationStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct OperationTrace {
    kind: OperationKind,
    started: std::time::Instant,
    context: String,
}

impl OperationTrace {
    pub fn start(kind: OperationKind, context: String) -> Self {
        OperationTrace {
            kind,
            started: std::time::Instant::now(),
            context,
        }
    }

    pub fn stage(&self, stage: OperationStage, detail: impl fmt::Display) {
        let elapsed = self.started.elapsed().as_millis();
        eprintln!(
            "[csswitch] {} | {} | {}ms | {} | {}",
            self.kind, stage, elapsed, self.context, detail
        );
    }

    pub fn finish(self, detail: impl fmt::Display) {
        let elapsed = self.started.elapsed().as_millis();
        eprintln!(
            "[csswitch] {} | done | {}ms | {} | {}",
            self.kind, elapsed, self.context, detail
        );
    }
}
