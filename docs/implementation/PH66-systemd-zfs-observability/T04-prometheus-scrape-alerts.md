# PH66 · T04 — Prometheus scrape config + Alertmanager rules

| Field | Value |
|---|---|
| **Phase** | PH66 — systemd + ZFS provisioning + Prometheus/Grafana |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/prometheus/calyx-scrape.yml`, `infra/aiwonder/alertmanager/calyx-alerts.yml` |
| **Depends on** | T03 (metrics endpoint live) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/16 §6` |

## Goal

Write the Prometheus scrape config snippet that adds `calyxd` as a target on the
existing aiwonder Prometheus instance (`127.0.0.1:9090`), and the Alertmanager
rules covering the five named alert conditions: recall tripwire breach,
chain-verify failure, guard FAR drift, lens endpoint down, and disk pressure on
`hotpool`. The Prometheus config is a drop-in snippet file (operator appends it
to the existing `prometheus.yml`). The Alertmanager rules file is self-contained
and references only Calyx metric names.

> **[OPERATOR] step:** applying the scrape config and reloading Prometheus
> (`sudo systemctl reload prometheus` or `POST /-/reload`) requires operator
> action. The operator appends `calyx-scrape.yml` to `prometheus.yml` and runs
> `curl -X POST http://127.0.0.1:9090/-/reload`.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/prometheus/calyx-scrape.yml` — drop-in scrape config:
  ```yaml
  # Append to /etc/prometheus/prometheus.yml under scrape_configs:
  # [OPERATOR] After appending: curl -X POST http://127.0.0.1:9090/-/reload
  - job_name: 'calyxd'
    static_configs:
      - targets: ['127.0.0.1:7700']
    metrics_path: '/metrics'
    scrape_interval: 15s
    scrape_timeout: 10s
    # calyx binds loopback only; Prometheus scrapes from the same host
  ```
- [ ] `infra/aiwonder/alertmanager/calyx-alerts.yml` — Alertmanager rules:
  ```yaml
  groups:
    - name: calyx
      rules:
        - alert: CalyxRecallTripwire
          expr: calyx_search_recall_tripwire == 0
          for: 2m
          labels: { severity: critical }
          annotations:
            summary: "Calyx recall tripwire tripped"
            description: "calyx_search_recall_tripwire is 0 — search recall has fallen below threshold"

        - alert: CalyxChainVerifyFail
          expr: calyx_ledger_chain_verify_ok == 0
          for: 0m
          labels: { severity: critical }
          annotations:
            summary: "Calyx ledger chain verification failed"
            description: "Ledger hash chain integrity check failed — possible data corruption"

        - alert: CalyxGuardFARDrift
          expr: calyx_guard_far > 0.05
          for: 5m
          labels: { severity: warning }
          annotations:
            summary: "Calyx guard FAR above 5% threshold"
            description: "Guard false-accept rate {{ $value }} exceeds 0.05 on {{ $labels.slot }}"

        - alert: CalyxLensEndpointDown
          expr: up{job="tei"} == 0
          for: 1m
          labels: { severity: critical }
          annotations:
            summary: "TEI lens endpoint down"
            description: "TEI endpoint {{ $labels.instance }} is not scraping"

        - alert: CalyxHotpoolDiskPressure
          expr: zfs_pool_free_bytes{pool="hotpool"} < 107374182400
          for: 5m
          labels: { severity: warning }
          annotations:
            summary: "hotpool < 100 GB free"
            description: "hotpool available: {{ $value | humanizeBytes }}; WAL may stall"
  ```
- [ ] Document in both files: "These configs do not modify Prometheus/Alertmanager
  in-place. The operator appends/copies them. No Calyx code reads or writes the
  Prometheus config — it is a one-time operator wiring step."
- [ ] Add `infra/aiwonder/prometheus/README.md` (≤50 lines) explaining the
  operator steps to apply the scrape config (append + reload) and add the
  Alertmanager rules file path to the Alertmanager `rule_files` stanza

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] YAML lint: `python3 -c "import yaml; yaml.safe_load(open('infra/aiwonder/prometheus/calyx-scrape.yml'))"` exits 0 (valid YAML)
- [ ] YAML lint: same for `calyx-alerts.yml`
- [ ] unit: `calyx-scrape.yml` contains `targets: ['127.0.0.1:7700']` (assert
  exact string — loopback only)
- [ ] unit: `calyx-alerts.yml` contains all five alert names: `CalyxRecallTripwire`,
  `CalyxChainVerifyFail`, `CalyxGuardFARDrift`, `CalyxLensEndpointDown`,
  `CalyxHotpoolDiskPressure`
- [ ] unit: no alert uses a threshold that would fire on a healthy system with
  default/zero metric values — assert `calyx_guard_far > 0.05` (not `> 0`) and
  `calyx_search_recall_tripwire == 0` (fires only when tripped, not on init)
- [ ] edge: the scrape config does not reference any port other than 7700 — grep
  for no `8080`, `9090`, `8088`, `8089`, `8090` in the calyx scrape snippet
  (those are other services, not calyxd)
- [ ] fail-closed: alert for `CalyxChainVerifyFail` has `for: 0m` (fires
  immediately, not after a delay) — assert in file

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Prometheus target status at `127.0.0.1:9090/api/v1/targets`; alert
  rule evaluation
- **Readback (after operator applies config):**
  ```bash
  # Verify calyxd target is UP in Prometheus:
  curl -s 'http://127.0.0.1:9090/api/v1/targets' \
    | python3 -c "import json,sys; t=json.load(sys.stdin); \
      [print(x['labels']['job'], x['health']) for x in t['data']['activeTargets']]"
  # Must show: calyxd up

  # Verify alert rules loaded:
  curl -s 'http://127.0.0.1:9090/api/v1/rules' \
    | python3 -c "import json,sys; r=json.load(sys.stdin); \
      [print(rule['name']) for g in r['data']['groups'] for rule in g['rules']]"
  # Must include all 5 CalyxXxx alert names
  ```
- **Prove:** `calyxd` appears in targets with `health: "up"`; all five alert
  rule names visible in rule list. API responses attached to PH66 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH66 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
