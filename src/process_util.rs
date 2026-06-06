/// Make this process a session leader so `pid == pgid` and MCP children can share the group.
#[cfg(unix)]
pub fn become_session_leader() {
    let rc = unsafe { libc::setsid() };
    if rc == -1 {
        tracing::warn!(
            error = %std::io::Error::last_os_error(),
            "setsid failed; MCP child cleanup may be incomplete"
        );
    } else {
        tracing::debug!(pgid = rc, "process session established");
    }
}

#[cfg(not(unix))]
pub fn become_session_leader() {}
