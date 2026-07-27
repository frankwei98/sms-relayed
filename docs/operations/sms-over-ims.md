# SMS over IMS diagnostics

The authenticated Modem page can show a best-effort `SMS over IMS` status. This
is auxiliary diagnostic information: it does not change the main modem health
and it is not queried by the public `/api/health` endpoint.

## What the status means

SmsRelayed asks the modem for three independent QMI values:

- IMSA SMS service status and its registration technology.
- IMSA registration status.
- The IMS SMS service enabled setting.

Runtime registration and service values determine the main status. The enabled
setting is explanatory only. `Available over WLAN` means the modem reports IMS
SMS service on WLAN or interworking WLAN. It does not prove the route used by an
individual message.

The first implementation supports only a direct QMI control port reported by
ModemManager, for example `/dev/wwan0qmi0`. It always uses `qmi-proxy`. It does
not probe QMI-over-MBIM, AT commands, or QRTR.

An `Unknown` result is safe and expected when qmicli is absent, its IMS actions
are missing, a QMI port cannot be selected unambiguously, the proxy is
unavailable, or the modem does not return recognized fields. A recognized
nonstandard label produces a warning while retaining the parsed result.

Some MSM8916/SD410 firmware enumerates vendor service IDs but rejects the
standard IMS and IMSA clients with QMI `InvalidServiceType`. SmsRelayed reports
this honestly as `Unknown` with fixed query-failure reasons; the service version
IDs alone are not treated as IMS evidence.

## Debian 12 and qmicli 1.36.0

Debian 12 ships qmicli 1.32.x, which does not expose the required IMS/IMSA
actions. The optional helper builds the official libqmi 1.36.0 source and
installs it privately:

```sh
sudo scripts/install-private-qmicli-debian.sh
```

The helper:

- verifies the pinned source archive with SHA-256;
- builds only direct-QMI support and installs it under
  `/opt/sms-relayed/libqmi-1.36.0`;
- atomically points `/opt/sms-relayed/libqmi` at that version;
- marks the version directory as helper-owned and refuses to overwrite
  conflicting operator-managed paths;
- writes
  `/etc/systemd/system/sms-relayed.service.d/qmicli.conf` with
  `SMS_RELAYED_QMICLI_PATH`;
- restarts `sms-relayed.service` only if it was active before installation;
- restores the previous symlink and drop-in if that restart fails.

It does not replace Debian's system libqmi or qmicli packages.

Use `--no-restart` to leave an active service running:

```sh
sudo scripts/install-private-qmicli-debian.sh --no-restart
```

Remove only the private version and managed systemd binding with:

```sh
sudo scripts/install-private-qmicli-debian.sh --uninstall
```

Uninstall refuses to recursively remove a version directory without the
helper's ownership marker. It stages the managed binding and restores it if
`daemon-reload` or the service restart fails.

## Verification

Check the private binary and service binding:

```sh
/opt/sms-relayed/libqmi/bin/qmicli-wrapper --version
/opt/sms-relayed/libqmi/bin/qmicli-wrapper --help-all |
  grep -E -- 'ims-get-ims-services-enabled-setting|imsa-get-ims-(registration|services)-status'
systemctl cat sms-relayed.service
systemctl status sms-relayed.service
```

For a direct QMI port such as `/dev/wwan0qmi0`, the three underlying diagnostic
queries are:

```sh
sudo /opt/sms-relayed/libqmi/bin/qmicli-wrapper \
  -d /dev/wwan0qmi0 --device-open-proxy \
  --imsa-get-ims-services-status
sudo /opt/sms-relayed/libqmi/bin/qmicli-wrapper \
  -d /dev/wwan0qmi0 --device-open-proxy \
  --imsa-get-ims-registration-status
sudo /opt/sms-relayed/libqmi/bin/qmicli-wrapper \
  -d /dev/wwan0qmi0 --device-open-proxy \
  --ims-get-ims-services-enabled-setting
```

Do not include raw command output in public diagnostics or logs; modem output
may contain network-specific details.
