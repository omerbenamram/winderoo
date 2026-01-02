//! Simple SNTP client helpers for embedded use.
//!
//! This module encodes/decodes NTP packets and provides an embassy-net based
//! client implementation suitable for ESP32.

use core::future::Future;
#[cfg(feature = "embedded")]
use alloc::boxed::Box;
#[cfg(feature = "embedded")]
use core::pin::Pin;

/// NTP timestamp delta between 1900-01-01 and 1970-01-01 in seconds.
pub const NTP_UNIX_DELTA: u64 = 2_208_988_800;

/// Errors returned by SNTP operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SntpError {
    /// The SNTP client failed to resolve or communicate.
    NetworkFailure,
    /// The SNTP response payload was invalid.
    InvalidResponse,
}

/// Async SNTP client abstraction.
pub trait SntpClient {
    /// Future returned by [`SntpClient::sync`].
    type Future<'a>: Future<Output = Result<u64, SntpError>>
    where
        Self: 'a;

    /// Fetch the current UTC epoch.
    fn sync<'a>(&'a mut self) -> Self::Future<'a>;
}

/// Build an SNTP request packet.
pub fn build_request_packet() -> [u8; 48] {
    let mut packet = [0u8; 48];
    // LI = 0, VN = 4, Mode = 3 (client)
    packet[0] = 0x23;
    packet
}

/// Parse an SNTP response packet and extract the UNIX epoch.
pub fn parse_response_packet(packet: &[u8]) -> Result<u64, SntpError> {
    if packet.len() < 48 {
        return Err(SntpError::InvalidResponse);
    }
    let seconds = u32::from_be_bytes([packet[40], packet[41], packet[42], packet[43]]) as u64;
    if seconds < NTP_UNIX_DELTA {
        return Err(SntpError::InvalidResponse);
    }
    Ok(seconds - NTP_UNIX_DELTA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_packet_sets_mode() {
        let packet = build_request_packet();
        assert_eq!(packet.len(), 48);
        assert_eq!(packet[0], 0x23);
    }

    #[test]
    fn parse_response_packet_extracts_unix_epoch() {
        let mut packet = [0u8; 48];
        let unix_epoch = 42u64;
        let ntp_seconds = unix_epoch + NTP_UNIX_DELTA;
        packet[40..44].copy_from_slice(&(ntp_seconds as u32).to_be_bytes());
        let parsed = parse_response_packet(&packet).expect("parse");
        assert_eq!(parsed, unix_epoch);
    }

    #[test]
    fn parse_response_packet_rejects_short_payload() {
        let packet = [0u8; 10];
        assert_eq!(parse_response_packet(&packet), Err(SntpError::InvalidResponse));
    }
}

/// Embassy-net backed SNTP client.
#[cfg(feature = "embedded")]
pub struct UdpSntpClient<'a, const RX: usize, const TX: usize> {
    socket: embassy_net::udp::UdpSocket<'a>,
    server: embassy_net::IpEndpoint,
    bound: bool,
}

#[cfg(feature = "embedded")]
impl<'a, const RX: usize, const TX: usize> UdpSntpClient<'a, RX, TX> {
    /// Create a new UDP SNTP client.
    pub fn new(
        stack: embassy_net::Stack<'a>,
        server: embassy_net::IpEndpoint,
        rx_meta: &'a mut [embassy_net::udp::PacketMetadata],
        rx_buffer: &'a mut [u8; RX],
        tx_meta: &'a mut [embassy_net::udp::PacketMetadata],
        tx_buffer: &'a mut [u8; TX],
    ) -> Self {
        let socket = embassy_net::udp::UdpSocket::new(
            stack,
            rx_meta,
            rx_buffer,
            tx_meta,
            tx_buffer,
        );
        Self {
            socket,
            server,
            bound: false,
        }
    }
}

#[cfg(feature = "embedded")]
impl<'a, const RX: usize, const TX: usize> SntpClient for UdpSntpClient<'a, RX, TX> {
    type Future<'b> = Pin<Box<dyn Future<Output = Result<u64, SntpError>> + 'b>> where Self: 'b;

    fn sync<'b>(&'b mut self) -> Self::Future<'b> {
        Box::pin(async move {
            if !self.bound {
                self.socket
                    .bind(0)
                    .map_err(|_| SntpError::NetworkFailure)?;
                self.bound = true;
            }

            let request = build_request_packet();
            self.socket
                .send_to(&request, self.server)
                .await
                .map_err(|_| SntpError::NetworkFailure)?;
            let mut response = [0u8; 48];
            let (len, _) = self
                .socket
                .recv_from(&mut response)
                .await
                .map_err(|_| SntpError::NetworkFailure)?;
            if len < response.len() {
                return Err(SntpError::InvalidResponse);
            }
            parse_response_packet(&response)
        })
    }
}
