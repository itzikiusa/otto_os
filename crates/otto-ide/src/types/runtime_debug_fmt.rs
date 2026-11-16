//! Redacted `std::fmt::Debug` implementations for runtime/collab wire structs
//! carrying environment variables or capability tokens.

use std::fmt;

use super::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeCollabJoinReq,
    IdeCollabJoinResp, IdeCreateCollabRoomResp, IdeDebugLaunchSpec,
};

impl fmt::Debug for IdeDebugLaunchSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let env_keys: Vec<&str> = self.env.keys().map(String::as_str).collect();
        f.debug_struct("IdeDebugLaunchSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("build_flags", &self.build_flags)
            .field("cwd", &self.cwd)
            .field("test_name", &self.test_name)
            .field("attach_port", &self.attach_port)
            .field("env_keys", &env_keys)
            .field("env_file", &self.env_file)
            .field("inherit_shell_env", &self.inherit_shell_env)
            .field("sdk_path", &self.sdk_path)
            .field("maven_profiles", &self.maven_profiles)
            .field("spring_profiles", &self.spring_profiles)
            .field("skip_tests", &self.skip_tests)
            .field("offline", &self.offline)
            .finish()
    }
}

impl fmt::Debug for IdeCreateCollabRoomResp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCreateCollabRoomResp")
            .field("room", &self.room)
            .field("invite_token", &"[REDACTED]")
            .field("share_hash", &"[REDACTED]")
            .field("host_peer_id", &self.host_peer_id)
            .field("host_peer_secret", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for IdeCollabJoinReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCollabJoinReq")
            .field("display_name", &self.display_name)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for IdeCollabJoinResp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCollabJoinResp")
            .field("peer_id", &self.peer_id)
            .field("peer_secret", &"[REDACTED]")
            .field("color", &self.color)
            .field("room", &self.room)
            .finish()
    }
}

impl fmt::Debug for IdeCollabCursorReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCollabCursorReq")
            .field("peer_id", &self.peer_id)
            .field("peer_secret", &"[REDACTED]")
            .field("path", &self.path)
            .field("selections", &self.selections)
            .field("active_line", &self.active_line)
            .finish()
    }
}

impl fmt::Debug for IdeCollabEditReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCollabEditReq")
            .field("peer_id", &self.peer_id)
            .field("peer_secret", &"[REDACTED]")
            .field("path", &self.path)
            .field("content_len", &self.content.len())
            .field("base_version", &self.base_version)
            .field("save", &self.save)
            .finish()
    }
}

impl fmt::Debug for IdeCollabDebugActionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdeCollabDebugActionReq")
            .field("peer_id", &self.peer_id)
            .field("peer_secret", &"[REDACTED]")
            .field("debug_id", &self.debug_id)
            .field("action", &self.action)
            .field("thread_id", &self.thread_id)
            .field("frame_id", &self.frame_id)
            .field("path", &self.path)
            .field("line", &self.line)
            .finish()
    }
}
