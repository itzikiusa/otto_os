//! Provider RPC ids are reusable. Public ids belong to one process generation,
//! including across daemon restarts, so an old card never answers a new request.
use otto_core::{new_id, Error, Result};

pub(super) struct RequestIds {
    prefix: String,
}

impl RequestIds {
    pub(super) fn new() -> Self {
        Self {
            prefix: format!("{}:", new_id()),
        }
    }

    pub(super) fn scoped(&self, native: &str) -> String {
        format!("{}{native}", self.prefix)
    }

    pub(super) fn native<'a>(&self, public: &'a str) -> Result<&'a str> {
        public.strip_prefix(&self.prefix).ok_or_else(|| {
            Error::Conflict("this request belongs to an earlier agent process".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_rpc_ids_never_target_a_replacement_process() {
        let old = RequestIds::new();
        let current = RequestIds::new();
        let a = old.scoped("rpc:1");
        let b = current.scoped("rpc:1");
        assert_ne!(a, b);
        assert_eq!(current.native(&b).unwrap(), "rpc:1");
        assert!(current.native(&a).is_err());
        assert!(current.native("rpc:1").is_err());
    }
}
