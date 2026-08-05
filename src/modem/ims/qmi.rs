use std::collections::HashMap;
use std::future::Future;
use std::io;
#[cfg(test)]
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub(super) const QMI_SERVICE_CTL: u8 = 0x00;
pub(super) const QMI_SERVICE_NAS: u8 = 0x03;
pub(super) const QMI_SERVICE_IMS: u8 = 0x12;
pub(super) const QMI_SERVICE_IMSA: u8 = 0x21;
const QMI_PROXY_OPEN: u16 = 0xff00;
pub(super) const QMI_CTL_GET_VERSION_INFO: u16 = 0x0021;
const QMI_CTL_ALLOCATE_CID: u16 = 0x0022;
const QMI_CTL_RELEASE_CID: u16 = 0x0023;
pub(super) const QMI_NAS_GET_SYSTEM_INFO: u16 = 0x004d;
pub(super) const QMI_IMSA_GET_REGISTRATION_STATUS: u16 = 0x0020;
pub(super) const QMI_IMSA_GET_SERVICES_STATUS: u16 = 0x0021;
pub(super) const QMI_IMS_GET_SERVICES_ENABLED: u16 = 0x0090;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeQmiError {
    PermissionDenied,
    ProxyUnavailable,
    Timeout,
    Protocol,
    Qmi(u16),
}

pub(super) trait QmiRequestClient: Send + Sync {
    fn request<'a>(
        &'a self,
        device: &'a str,
        service: u8,
        message: u16,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, NativeQmiError>> + Send + 'a>>;
}

#[derive(Clone)]
enum QmiProxyEndpoint {
    Abstract,
    #[cfg(test)]
    Filesystem(PathBuf),
}

#[derive(Clone)]
pub(super) struct ProxyQmiClient {
    endpoint: QmiProxyEndpoint,
}

impl ProxyQmiClient {
    pub(super) fn new() -> Self {
        Self {
            endpoint: QmiProxyEndpoint::Abstract,
        }
    }

    #[cfg(test)]
    pub(super) fn with_socket_path(path: PathBuf) -> Self {
        Self {
            endpoint: QmiProxyEndpoint::Filesystem(path),
        }
    }

    async fn request_inner(
        &self,
        device: &str,
        service: u8,
        message: u16,
        cleanup_timeout: Duration,
    ) -> Result<Vec<u8>, NativeQmiError> {
        let mut stream = self.connect().await?;
        let open = build_qmi_request(
            QMI_SERVICE_CTL,
            0,
            1,
            QMI_PROXY_OPEN,
            &[(0x01, device.as_bytes())],
        )?;
        stream.write_all(&open).await.map_err(map_proxy_io_error)?;
        let open_response = read_qmi_frame(&mut stream).await?;
        QmiResponse::parse(&open_response, QMI_SERVICE_CTL, QMI_PROXY_OPEN)?;

        if service == QMI_SERVICE_CTL {
            let request = build_qmi_request(service, 0, 2, message, &[])?;
            stream
                .write_all(&request)
                .await
                .map_err(map_proxy_io_error)?;
            let response = read_qmi_frame(&mut stream).await?;
            QmiResponse::parse(&response, service, message)?;
            return Ok(response);
        }

        let allocate = build_qmi_request(
            QMI_SERVICE_CTL,
            0,
            2,
            QMI_CTL_ALLOCATE_CID,
            &[(0x01, &[service])],
        )?;
        stream
            .write_all(&allocate)
            .await
            .map_err(map_proxy_io_error)?;
        let allocate_response = read_qmi_frame(&mut stream).await?;
        let allocation =
            QmiResponse::parse(&allocate_response, QMI_SERVICE_CTL, QMI_CTL_ALLOCATE_CID)?;
        let allocated = allocation.tlvs.get(&0x01).ok_or(NativeQmiError::Protocol)?;
        if allocated.len() != 2 || allocated[0] != service {
            return Err(NativeQmiError::Protocol);
        }
        let client = allocated[1];

        let service_request = build_qmi_request(service, client, 1, message, &[])?;
        stream
            .write_all(&service_request)
            .await
            .map_err(map_proxy_io_error)?;
        let service_response = read_qmi_frame(&mut stream).await.and_then(|response| {
            QmiResponse::parse(&response, service, message)?;
            Ok(response)
        });

        let release = build_qmi_request(
            QMI_SERVICE_CTL,
            0,
            3,
            QMI_CTL_RELEASE_CID,
            &[(0x01, &[service, client])],
        );
        if let Ok(release) = release {
            tokio::spawn(async move {
                let cleanup = async {
                    stream
                        .write_all(&release)
                        .await
                        .map_err(map_proxy_io_error)?;
                    let response = read_qmi_frame(&mut stream).await?;
                    QmiResponse::parse(&response, QMI_SERVICE_CTL, QMI_CTL_RELEASE_CID)?;
                    Ok::<(), NativeQmiError>(())
                };
                let _ = tokio::time::timeout(cleanup_timeout, cleanup).await;
            });
        }

        service_response
    }

    async fn connect(&self) -> Result<UnixStream, NativeQmiError> {
        match &self.endpoint {
            #[cfg(test)]
            QmiProxyEndpoint::Filesystem(path) => {
                UnixStream::connect(path).await.map_err(map_proxy_io_error)
            }
            QmiProxyEndpoint::Abstract => connect_abstract_qmi_proxy().await,
        }
    }
}

impl Default for ProxyQmiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl QmiRequestClient for ProxyQmiClient {
    fn request<'a>(
        &'a self,
        device: &'a str,
        service: u8,
        message: u16,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, NativeQmiError>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::timeout(
                timeout,
                self.request_inner(device, service, message, timeout),
            )
            .await
            .map_err(|_| NativeQmiError::Timeout)?
        })
    }
}

fn build_qmi_request(
    service: u8,
    client: u8,
    transaction: u16,
    message: u16,
    tlvs: &[(u8, &[u8])],
) -> Result<Vec<u8>, NativeQmiError> {
    let mut frame = vec![0x01, 0x00, 0x00, 0x00, service, client, 0x00];
    if service == QMI_SERVICE_CTL {
        frame.push(u8::try_from(transaction).map_err(|_| NativeQmiError::Protocol)?);
    } else {
        frame.extend_from_slice(&transaction.to_le_bytes());
    }
    frame.extend_from_slice(&message.to_le_bytes());
    let tlv_length_offset = frame.len();
    frame.extend_from_slice(&[0x00, 0x00]);
    let tlv_offset = frame.len();
    for (kind, value) in tlvs {
        let length = u16::try_from(value.len()).map_err(|_| NativeQmiError::Protocol)?;
        frame.push(*kind);
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(value);
    }
    let tlv_length =
        u16::try_from(frame.len() - tlv_offset).map_err(|_| NativeQmiError::Protocol)?;
    frame[tlv_length_offset..tlv_length_offset + 2].copy_from_slice(&tlv_length.to_le_bytes());
    let qmux_length = u16::try_from(frame.len() - 1).map_err(|_| NativeQmiError::Protocol)?;
    frame[1..3].copy_from_slice(&qmux_length.to_le_bytes());
    Ok(frame)
}

async fn read_qmi_frame(stream: &mut UnixStream) -> Result<Vec<u8>, NativeQmiError> {
    let mut header = [0_u8; 3];
    stream
        .read_exact(&mut header)
        .await
        .map_err(map_proxy_io_error)?;
    if header[0] != 0x01 {
        return Err(NativeQmiError::Protocol);
    }
    let total = usize::from(u16::from_le_bytes([header[1], header[2]])) + 1;
    if total < header.len() {
        return Err(NativeQmiError::Protocol);
    }
    let mut frame = vec![0_u8; total];
    frame[..header.len()].copy_from_slice(&header);
    stream
        .read_exact(&mut frame[header.len()..])
        .await
        .map_err(map_proxy_io_error)?;
    Ok(frame)
}

fn map_proxy_io_error(error: io::Error) -> NativeQmiError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => NativeQmiError::PermissionDenied,
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
            NativeQmiError::ProxyUnavailable
        }
        io::ErrorKind::TimedOut => NativeQmiError::Timeout,
        _ => NativeQmiError::Protocol,
    }
}

#[cfg(target_os = "linux")]
async fn connect_abstract_qmi_proxy() -> Result<UnixStream, NativeQmiError> {
    tokio::task::spawn_blocking(|| {
        use std::mem::{offset_of, zeroed};
        use std::os::fd::FromRawFd;

        const SOCKET_NAME: &[u8] = b"qmi-proxy";
        let file_descriptor =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if file_descriptor < 0 {
            return Err(map_proxy_io_error(io::Error::last_os_error()));
        }
        let mut address: libc::sockaddr_un = unsafe { zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (target, source) in address.sun_path[1..].iter_mut().zip(SOCKET_NAME) {
            *target = *source as libc::c_char;
        }
        let address_length = offset_of!(libc::sockaddr_un, sun_path) + 1 + SOCKET_NAME.len();
        let connected = unsafe {
            libc::connect(
                file_descriptor,
                std::ptr::addr_of!(address).cast(),
                address_length as libc::socklen_t,
            )
        };
        if connected != 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(file_descriptor) };
            return Err(map_proxy_io_error(error));
        }
        let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(file_descriptor) };
        stream.set_nonblocking(true).map_err(map_proxy_io_error)?;
        UnixStream::from_std(stream).map_err(map_proxy_io_error)
    })
    .await
    .map_err(|_| NativeQmiError::Protocol)?
}

#[cfg(not(target_os = "linux"))]
async fn connect_abstract_qmi_proxy() -> Result<UnixStream, NativeQmiError> {
    Err(NativeQmiError::ProxyUnavailable)
}

#[derive(Debug)]
pub(super) struct QmiResponse {
    pub(super) tlvs: HashMap<u8, Vec<u8>>,
}

impl QmiResponse {
    pub(super) fn parse(
        raw: &[u8],
        expected_service: u8,
        expected_message: u16,
    ) -> Result<Self, NativeQmiError> {
        if raw.len() < 12 || raw[0] != 0x01 {
            return Err(NativeQmiError::Protocol);
        }
        let declared = usize::from(u16::from_le_bytes([raw[1], raw[2]])) + 1;
        if declared != raw.len() || raw[3] & 0x80 == 0 || raw[4] != expected_service {
            return Err(NativeQmiError::Protocol);
        }

        let (flags, message_offset, tlv_length_offset, tlv_offset, response_flag) =
            if expected_service == QMI_SERVICE_CTL {
                (raw[6], 8, 10, 12, 0x01)
            } else {
                if raw.len() < 13 {
                    return Err(NativeQmiError::Protocol);
                }
                (raw[6], 9, 11, 13, 0x02)
            };
        if flags & response_flag == 0
            || u16::from_le_bytes([raw[message_offset], raw[message_offset + 1]])
                != expected_message
        {
            return Err(NativeQmiError::Protocol);
        }

        let tlv_length = usize::from(u16::from_le_bytes([
            raw[tlv_length_offset],
            raw[tlv_length_offset + 1],
        ]));
        if tlv_offset + tlv_length != raw.len() {
            return Err(NativeQmiError::Protocol);
        }

        let mut tlvs = HashMap::new();
        let mut offset = tlv_offset;
        while offset < raw.len() {
            if offset + 3 > raw.len() {
                return Err(NativeQmiError::Protocol);
            }
            let kind = raw[offset];
            let length = usize::from(u16::from_le_bytes([raw[offset + 1], raw[offset + 2]]));
            offset += 3;
            let end = offset.checked_add(length).ok_or(NativeQmiError::Protocol)?;
            if end > raw.len() || tlvs.insert(kind, raw[offset..end].to_vec()).is_some() {
                return Err(NativeQmiError::Protocol);
            }
            offset = end;
        }

        let result = tlvs.get(&0x02).ok_or(NativeQmiError::Protocol)?;
        if result.len() != 4 {
            return Err(NativeQmiError::Protocol);
        }
        let failed = u16::from_le_bytes([result[0], result[1]]);
        let error = u16::from_le_bytes([result[2], result[3]]);
        if failed != 0 {
            return Err(NativeQmiError::Qmi(error));
        }
        Ok(Self { tlvs })
    }

    pub(super) fn bool(&self, kind: u8) -> Option<bool> {
        self.tlvs
            .get(&kind)
            .filter(|value| value.len() == 1)
            .map(|value| value[0] != 0)
    }

    pub(super) fn u32(&self, kind: u8) -> Option<u32> {
        self.tlvs
            .get(&kind)
            .filter(|value| value.len() == 4)
            .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
    }
}
