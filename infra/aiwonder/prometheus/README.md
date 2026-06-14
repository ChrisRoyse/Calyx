# Calyx Prometheus / Alertmanager wiring (PH66 T04)

Repo-owned config the **operator** applies to the existing aiwonder Prometheus
(`127.0.0.1:9090`). Nothing here is read or written by Calyx code — these are
one-time wiring steps.

## 1. Scrape config — `calyx-scrape.yml`

Append the `calyxd` job under `scrape_configs:` in
`/etc/prometheus/prometheus.yml`, then reload:

```bash
curl -X POST http://127.0.0.1:9090/-/reload
# verify: target calyxd is up
curl -s 'http://127.0.0.1:9090/api/v1/targets' \
  | python3 -c "import json,sys; [print(x['labels']['job'],x['health']) for x in json.load(sys.stdin)['data']['activeTargets']]"
```

## 2. Alert rules — `../alertmanager/calyx-alerts.yml`

Copy the file and add its path to the Prometheus `rule_files:` stanza, then
reload as above. Verify the ZFS and Calyx rules loaded:

```bash
curl -s 'http://127.0.0.1:9090/api/v1/rules' \
  | python3 -c "import json,sys; [print(r['name']) for g in json.load(sys.stdin)['data']['groups'] for r in g['rules']]"
# expect includes: CalyxRecallTripwire CalyxChainVerifyFail CalyxGuardFARDrift
#                  CalyxLensEndpointDown CalyxHotpoolDiskPressure
#                  CalyxZfsPoolUnhealthy CalyxZfsChecksumErrors
#                  CalyxZfsScrubStale CalyxZfsDatasetChecksumDisabled
```

## Validation before applying

```bash
promtool check rules calyx-alerts.yml          # syntax
promtool test rules  calyx-alerts.test.yml     # synthetic series -> expected alerts
```

`CalyxLensEndpointDown` uses `up{job="tei"}` and `CalyxHotpoolDiskPressure` uses
`zfs_pool_free_bytes{pool="hotpool"}` — sourced from the TEI scrape job and the
ZFS exporter, not calyxd. The `CalyxZfs*` integrity alerts are sourced from the
calyxd metrics that read `zfs get checksum` and `zpool status` directly. If the
TEI job is named differently in `prometheus.yml`, adjust the `job="tei"` selector
to match.
