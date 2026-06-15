# Stage 17 — Scale: DiskANN + SPANN (PH68)

Take the server vault from 1e6–1e7 (in-RAM HNSW) to 1e8–1e9 constellations with
disk-resident graph indexes and tiered sparse posting lists, within the search
SLO. Lands in `calyx-sextant`. Be explicit (PRD `17 §3.4`): billion-scale is a
**server** target on aiwonder's NVMe+HDD, never a laptop/embedded promise.

---

## PH68 — DiskANN dense + SPANN sparse
- **Objective.** Disk-resident dense ANN (DiskANN) + memory/disk hybrid sparse
  posting lists (SPANN) so billion-scale stays within the search SLO.
- **Deps.** PH23 (HNSW), PH25 (inverted).
- **Deliverables.** `index/diskann` (on-disk graph, beamwidth-tuned, vectors
  co-located for locality), `index/spann` (centroids in RAM, posting lists on
  NVMe), dual DiskANN for asymmetric slots, kernel-first funnel for huge vaults
  (kernel-of-regions → region → cx).
- **Key tasks.** build on aiwonder's `hotpool` NVMe; beamwidth/posting-cutoff
  autotuned by Anneal; the 3-hop funnel for 1e8+ vaults; raw-f32 rescore from
  cold sidecar.
- **FSV gate.** a **1e8–1e9-cx server vault** on aiwonder answers within the
  search SLO (KernelFirst@1e8 p99 < 25 ms target); DiskANN graph + SPANN lists
  physically on disk (verify paths + measured latency).
- **Axioms/PRD.** P10, `10 §3`, `04 §8`, `19 §4`.

---

## Stage 17 exit
Calyx serves billion-scale server vaults within the search SLO via disk-resident
graphs and tiered sparse lists, with the kernel funnel keeping huge-vault queries
sublinear — PRD `SCALE`.
