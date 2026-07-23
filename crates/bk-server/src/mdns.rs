//! mDNS announcer — registers `talon-server._talon._tcp.local.`
//! on the LAN when remote mode is on. The announcer is a
//! guard: on `Drop` it unregisters the service so the
//! announcement ends when the server exits cleanly.

use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// The mDNS service type for the Talon server. `_talon` is
/// the protocol family (Talon-specific), `_tcp` is the
/// transport (TCP over the underlying TLS-wrapped HTTP
/// connection).
const SERVICE_TYPE: &str = "_talon._tcp.local.";

/// The mDNS guard. On `Drop` it shuts down the service
/// daemon, which unregisters the service.
pub struct MdnsAnnouncer {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAnnouncer {
    /// Register `instance_name` (e.g. `talon-server`) on the
    /// LAN with the given port. Returns an error if the
    /// registration fails (the server logs a warning and
    /// continues without discovery).
    ///
    /// The TXT record carries the URL (`https://host:port/`)
    /// so the discovered client knows where to point. The
    /// auth token is NEVER in the TXT record (the user must
    /// copy it via `talon token`) — auto-distributing the
    /// token via mDNS would be a security smell.
    pub fn new(instance_name: &str, port: u16) -> Result<Self, crate::ServerError> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| crate::ServerError::Mdns(format!("ServiceDaemon::new: {e}")))?;
        // The hostname is left empty; the daemon fills it
        // in from the local interfaces. The IP is also
        // auto-discovered (the `0.0.0.0` sentinel means
        // "all interfaces"). The TXT record carries the
        // URL so the discovered client knows where to
        // point.
        let url = format!("https://localhost:{port}/");
        let properties: &[(&str, &str)] = &[("url", &url)];
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            "", // host_name (auto)
            IpAddr::from([0, 0, 0, 0]),
            port,
            properties,
        )
        .map_err(|e| crate::ServerError::Mdns(format!("ServiceInfo::new: {e}")))?;
        let fullname = service_info.get_fullname().to_string();
        daemon
            .register(service_info)
            .map_err(|e| crate::ServerError::Mdns(format!("register: {e}")))?;
        tracing::info!(fullname = %fullname, port, "mDNS service registered");
        Ok(Self { daemon, fullname })
    }

    /// The full service name (e.g.
    /// `talon-server._talon._tcp.local.`).
    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for MdnsAnnouncer {
    fn drop(&mut self) {
        // Best-effort: try to unregister the service. The
        // shutdown of the daemon also unregisters
        // everything it knows about.
        if let Err(e) = self.daemon.shutdown() {
            tracing::warn!(fullname = %self.fullname, "mDNS shutdown failed: {e}");
        }
    }
}
