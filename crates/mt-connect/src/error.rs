use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectError {
    /// No stored session, or the server revoked it (refresh token rejected).
    #[error("You are not signed in to Voltrus.")]
    NotSignedIn,
    /// Sign-in flow problems the user can act on (cancelled, stale, mismatched redirect).
    #[error("{0}")]
    Auth(String),
    /// A redirect whose `state` is not ours: another app or a forged link, never the sign-in we started.
    #[error("The sign-in response did not match this request.")]
    ForeignCallback,
    /// The server could not be reached. Stored tokens are kept.
    #[error("Could not reach Voltrus: {0}")]
    Network(String),
    /// The server answered with an error.
    #[error("{message}")]
    Api { status: u16, code: Option<String>, message: String },
    #[error("{0}")]
    Store(String),
    #[error("{0}")]
    Internal(String),
}

impl ConnectError {
    /// `invalid_grant` and friends: the refresh token is no longer usable.
    pub fn is_grant_rejected(&self) -> bool {
        matches!(self, ConnectError::Api { status, code, .. }
            if matches!(*status, 400 | 401) && code.as_deref().is_none_or(|c| c == "invalid_grant" || c == "invalid_token"))
    }
}

impl From<ConnectError> for String {
    fn from(e: ConnectError) -> String {
        e.to_string()
    }
}
