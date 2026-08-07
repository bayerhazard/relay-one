use serde::Serialize;
use std::fmt;
use thiserror::Error;

/// Structured context attached to every AppError variant.
///
/// Provides operation name, affected account, and folder for
/// meaningful error diagnostics and structured logging.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorContext {
    /// Short operation tag, e.g. "ssl_connect", "uid_fetch", "send"
    pub operation: &'static str,
    /// Affected account ID, if available at the error site
    pub account_id: Option<u32>,
    /// Affected folder name, if relevant (IMAP operations)
    pub folder: Option<String>,
}

impl ErrorContext {
    pub fn new(operation: &'static str) -> Self {
        Self {
            operation,
            account_id: None,
            folder: None,
        }
    }

    pub fn with_account(mut self, account_id: u32) -> Self {
        self.account_id = Some(account_id);
        self
    }

    pub fn with_folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = Some(folder.into());
        self
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.operation)?;
        if let Some(account_id) = self.account_id {
            write!(f, " account={}", account_id)?;
        }
        if let Some(ref folder) = self.folder {
            write!(f, " folder={}", folder)?;
        }
        Ok(())
    }
}

#[derive(Error, Debug, Clone, Serialize)]
pub enum AppError {
    #[error("[{ctx}] IMAP: {msg}")]
    Imap {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] SMTP: {msg}")]
    Smtp {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] Cache: {msg}")]
    Cache {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] AI: {msg}")]
    Ai {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] Network: {msg}")]
    Network {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] Auth: {msg}")]
    Auth {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
    #[error("[{ctx}] Not found: {msg}")]
    NotFound {
        msg: String,
        #[serde(skip)]
        ctx: ErrorContext,
    },
}

// ── Constructor helpers with automatic structured tracing ─────

impl AppError {
    /// Create an IMAP error with structured context and a `warn!` log.
    pub fn imap(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "IMAP",
            operation,
            message = %msg,
            "[{operation}] IMAP: {msg}"
        );
        AppError::Imap {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Create an SMTP error with structured context and a `warn!` log.
    pub fn smtp(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "SMTP",
            operation,
            message = %msg,
            "[{operation}] SMTP: {msg}"
        );
        AppError::Smtp {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Create a Cache error with structured context and a `warn!` log.
    pub fn cache(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "Cache",
            operation,
            message = %msg,
            "[{operation}] Cache: {msg}"
        );
        AppError::Cache {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Create an Auth error with structured context and a `warn!` log.
    pub fn auth(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "Auth",
            operation,
            message = %msg,
            "[{operation}] Auth: {msg}"
        );
        AppError::Auth {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Create a NotFound error with structured context and a `warn!` log.
    pub fn not_found(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "NotFound",
            operation,
            message = %msg,
            "[{operation}] Not found: {msg}"
        );
        AppError::NotFound {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Create an AI error with structured context and a `warn!` log.
    pub fn ai(msg: impl Into<String>, operation: &'static str) -> Self {
        let msg = msg.into();
        tracing::warn!(
            target: "AppError",
            error_type = "AI",
            operation,
            message = %msg,
            "[{operation}] AI: {msg}"
        );
        AppError::Ai {
            msg,
            ctx: ErrorContext::new(operation),
        }
    }

    /// Attach an `account_id` to this error's context.
    pub fn with_account(mut self, account_id: u32) -> Self {
        if let Some(ctx) = self.ctx_mut() {
            ctx.account_id = Some(account_id);
        }
        self
    }

    /// Attach a `folder` name to this error's context.
    pub fn with_folder(mut self, folder: impl Into<String>) -> Self {
        if let Some(ctx) = self.ctx_mut() {
            ctx.folder = Some(folder.into());
        }
        self
    }

    fn ctx_mut(&mut self) -> Option<&mut ErrorContext> {
        match self {
            AppError::Imap { ctx, .. }
            | AppError::Smtp { ctx, .. }
            | AppError::Cache { ctx, .. }
            | AppError::Ai { ctx, .. }
            | AppError::Network { ctx, .. }
            | AppError::Auth { ctx, .. }
            | AppError::NotFound { ctx, .. } => Some(ctx),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::cache(e.to_string(), "database")
    }
}
