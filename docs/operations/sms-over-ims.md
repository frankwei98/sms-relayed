# Native IMS, VoLTE, and VoWiFi diagnostics

The authenticated Modem page includes a read-only IMS probe implemented inside
SmsRelayed. It does not execute or parse `qmicli`, and it does not change the
main modem health or the public `/api/health` result.

## Evidence levels

The probe deliberately keeps three different claims separate:

- **Capability:** NAS `Get System Info` may report LTE voice support and IMS
  voice support. These flags say that the modem/network combination advertises
  the capability; they do not prove IMS registration or an active call route.
- **Configuration:** IMS `Get Services Enabled` may report separate VoLTE,
  VoWiFi, and SMS enable flags. An enabled flag is not proof that the service is
  registered or currently usable.
- **Runtime state:** IMSA registration, voice/SMS service status, and access
  technology are the strongest available modem evidence.

SmsRelayed reports **VoLTE active** only when IMSA reports all of the following:

1. IMS is registered;
2. voice service is available; and
3. the voice access technology is WWAN.

It reports **VoWiFi active** only when the first two conditions hold and the
voice access technology is WLAN or interworking WLAN. Missing or contradictory
fields produce `Unknown`, `Limited`, or another conservative state instead of a
positive claim.

SMS over IMS remains a separate status. Even an available IMS SMS service does
not prove the route used by each individual message.

## Transport

SmsRelayed implements the required QMUX framing, qmi-proxy handshake, CTL client
ID lifecycle, response validation, and these read-only QMI requests:

- CTL `Get Version Info` for service discovery;
- NAS `Get System Info` for LTE/IMS voice capability;
- IMSA `Get Registration Status` and `Get Services Status` for runtime state;
- IMS `Get Services Enabled` for configuration evidence.

The current transport supports a direct QMI control port reported by
ModemManager, such as `/dev/wwan0qmi0`, and shares it through the existing
abstract `qmi-proxy` socket. It does not yet support QMI-over-MBIM, QRTR, or AT
command fallbacks.

## Interpreting `Unknown`

`Unknown` is expected when:

- ModemManager reports no unambiguous QMI port;
- qmi-proxy is unavailable or access is denied;
- the modem omits the IMS (`0x12`) or IMSA (`0x21`) standard services;
- a service rejects the request or returns an unrecognized response; or
- the five-second probe budget expires.

Some MSM8916/SD410 firmware reports NAS IMS voice capability but does not expose
the standard IMS/IMSA services. In that case SmsRelayed preserves the capability
claim while leaving VoLTE/VoWiFi runtime state `Unknown`. Service IDs or enabled
settings alone are never promoted to runtime evidence.

## Enabling services

This probe is intentionally read-only. Enabling VoLTE or VoWiFi can require a
carrier-specific modem profile, credentials, provisioning, and persistent NV or
PDC changes. A generic write can break cellular registration, so SmsRelayed does
not expose an enable action until a modem-specific transaction can be identified,
validated, and rolled back safely.

Do not include raw QMI frames in public diagnostics or logs; responses may
contain network-specific details.
