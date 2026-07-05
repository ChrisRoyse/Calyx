# #1194 GPU/Sparse Association Acceleration Decision

## Scope

#1194 evaluates whether sparse graph association mining needs a GPU path now
that the large biomedical graph readers use persisted binary CSR. The target
surface is graph association work such as spectral communities, PPR/path-style
walk scoring, and bridge mining. This is a performance decision only; it does
not make a biomedical, treatment, safety, or cure claim.

## Decision

No GPU sparse graph kernel is selected for the current #867 path.

Reason: after #1191/#1210/#1213, the full #869 graph has a persisted binary CSR
and the current CPU/Rayon spectral-community run over the real graph completes
in 8.581 seconds from source-of-truth bytes. There is no existing Calyx sparse
graph GPU backend to verify without adding a speculative dependency, and the
measured current workload does not justify wiring a CUDA sparse eigensolver/PPR
backend ahead of the open higher-value GPU lens/resident issues (#1155, #1156,
#1158, #1159).

GPU graph kernels should be revisited only if a larger measured workload shows
a dominant sparse-matvec/PPR/path bottleneck and the implementation can be
verified against the CPU output hash with a strict tolerance.

## Real FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z
```

Profile summary:

```text
/home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/profile_summary.json
sha256: 7fb933a37f6cd6873879021e1ae62519a461e80f36adae7143cceb52c828f11b
```

Hardware readback:

```text
GPU: NVIDIA GeForce RTX 5090
Driver: 610.43.02
Memory: 32607 MiB
CUDA toolkit: 13.3.33
```

Benchmark input:

| Field | Value |
|---|---:|
| Vault | `corpus-anchored-869-20260625T080546Z` |
| Vault id | `01KVYX0KYVBQSGVC6N2S00FX6J` |
| Collection | `default` |
| Nodes | 198,993 |
| CSR edges | 2,435,817 |
| Association edge count | 2,435,817 |
| CSR bytes | 63,235,522 |
| CSR SHA-256 | `39124a1f244d6838360fccb8a43f62771b8d62d34bfcaf1c575d30c5d59df6a8` |

CSR command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

CPU profile command:

```bash
/usr/bin/time -v env CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 \
  ./target/release/calyx spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 3 \
  --eigen-max-iter 64 \
  --centrality-max-iter 512 \
  --centrality-tol 0.00001 \
  --max-bridge-candidates 8 \
  --max-centrality-candidates 8 \
  --out /home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/cpu_spectral_report.json
```

GPU telemetry command:

```bash
nvidia-smi --query-gpu=timestamp,utilization.gpu,memory.used,power.draw \
  --format=csv -l 1
```

## Output Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `csr_stdout.json` | 940 | `5663ec001560bc4b548a3e99d82899ce4496a5fd9afb3d8925d01afe864714ba` |
| `csr_stderr.txt` | 758 | `5c147c791b30707a910e5efd9aed7bc65bcf5c8db7119c3c1855d706669fb4b3` |
| `cpu_spectral_report.json` | 42,817,296 | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `cpu_spectral_stdout.json` | 34,473,492 | `83fd98bdde79318bb344e3ecb4465fe556cdbd662a0738c7890f294e5cc55cd5` |
| `cpu_spectral_stderr.txt` | 1,989 | `d9a1006e779d3ac5ab284d972c9768feb5f1f12f061abe01c9c2daa42114c39d` |
| `cpu_spectral_exit.txt` | 2 | `9a271f2a916b0b6ee6cecb2426f0b3206ef074578be55d9bc94f6f3fe3ab86aa` |
| `gpu_spectral_samples.csv` | 507 | `8dd2cd3eaedd7ae0a407cf9abffd4a9e851f77c8ac3ebd50a7778397e2be8c50` |
| `invalid_eigen_stdout.json` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `invalid_eigen_stderr.txt` | 150 | `1d7d3c3b613f2585f40fc3b4e466178ea10409935aef176ed84e17f2635fb639` |

## CPU Profile

| Metric | Value |
|---|---:|
| Exit status | 0 |
| Persisted CSR loaded | true |
| Graph nodes | 198,993 |
| Graph edges | 2,435,817 |
| Communities | 2 |
| Bridge candidates | 8 |
| Centrality candidates | 8 |
| Spectral gap | 0.943023681640625 |
| CLI elapsed | 8,581 ms |
| `/usr/bin/time` wall clock | 0:08.99 |
| CPU percent | 447 |
| User seconds | 37.94 |
| System seconds | 2.29 |
| Max RSS | 5,976,604 KiB |

## GPU Telemetry

| Metric | Value |
|---|---:|
| Samples | 9 |
| Max GPU utilization | 0% |
| Max memory used | 10,212 MiB |
| Max power draw | 65.14 W |

GPU output hash:

```text
null
```

Parity status:

```text
not_applicable_no_gpu_sparse_backend_selected
```

This is intentional: no GPU kernel was selected because the measured current
CPU path is below the threshold where speculative GPU work is justified, and
there is no existing Calyx graph-GPU backend to run as a parity candidate.

## Failure Case

Invalid eigen count:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 1 \
  --out /home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/invalid_eigen_report.json
```

Readback:

| Field | Value |
|---|---:|
| Exit status | 2 |
| Error code | `CALYX_CLI_USAGE_ERROR` |
| Invalid report exists | false |

## Assertions

| Assertion | Value |
|---|---:|
| CSR readback is ok | true |
| CPU spectral command exit is zero | true |
| CPU report SHA matches CLI stdout artifact SHA | true |
| Profiled real large graph | true |
| Spectral reader loaded persisted CSR | true |
| GPU samples captured | true |
| GPU sparse kernel not selected | true |
| Invalid eigen case failed closed | true |

## Conclusion

#1194 is resolved as a measured no-GPU decision for the current association
graph path. The persisted binary CSR path removes the prior row-scan bottleneck,
the real full-graph spectral run is CPU/Rayon-fast enough for the current #867
workflow, and no speculative CUDA sparse graph backend is added.

Future work belongs in a new performance issue only when a larger real workload
produces a measured sparse-matvec/PPR/path-scoring bottleneck and a GPU kernel
can be verified against the CPU output hash with explicit tolerances.
