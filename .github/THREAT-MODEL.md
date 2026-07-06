# GitHub Actions Threat Model

## Purpose

This document is the threat model for the CI/CD automation in this repository's
[`.github/workflows`](./workflows) directory. It exists so that anyone changing
a workflow can see, at a glance, which threats were considered and how the
current design mitigates them — and can re-evaluate those decisions when the
automation changes.

It is structured around Adam Shostack's four framing questions:

1. **What are we building?** — Each workflow is described by its trigger,
   privileges, and the code it executes (see the section per workflow below).
2. **What can go wrong?** — Threats are enumerated using **STRIDE** (Spoofing,
   Tampering, Repudiation, Information disclosure, Denial of service, Elevation
   of privilege), specialized to the GitHub Actions execution model.
3. **What are we going to do about it?** — Each threat has an explicit
   mitigation implemented in the workflow.
4. **Did we do a good job?** — The tables are the review checklist; the
   "Residual risk & assumptions" section records what is deliberately *not*
   mitigated and must hold true for this model to be valid.

There is one section per workflow. Each contains a table of
`STRIDE category / threat` → `how it applies to this workflow` → `mitigation`.

---

## Workflow: `run-ci`

**What it is.** [`run-ci.yml`](./workflows/run-ci.yml) type-checks, format-checks,
lints, and performs a no-bundle release build of the Tauri application on
`macos-latest` and `windows-latest`.

**Trigger & privilege surface.**
- Triggers: `pull_request` (opened/synchronize/reopened) gated to head branches
  prefixed `fix/` or `feat/`; `workflow_call`; `workflow_dispatch`.
- Token: default-deny (`permissions: {}`), with the single job granted only
  `contents: read`.
- Code executed: the PR's own source (potentially untrusted, for fork PRs),
  plus the pinned official actions and the runner's pre-installed `rustup`.

**Trust boundary.** The primary boundary is *untrusted PR-authored code* (build
scripts, `run.sh`, Cargo/npm dependencies, `tauri build` output) running on a
GitHub-hosted runner that also holds the job's `GITHUB_TOKEN`. The design goal
is that crossing this boundary yields nothing worth stealing and no ability to
mutate the repository.

| STRIDE — Threat | How it applies to `run-ci` | Mitigation |
|---|---|---|
| **Spoofing** — Action/tag hijacking or a compromised maintainer republishing a mutable tag (supply-chain impersonation, e.g. `tj-actions/changed-files`). | The workflow consumes `actions/checkout` and `actions/setup-node`; a repointed `@v4` tag would execute attacker code with job access. | Only **official `actions/*`** are used, each **pinned to a full commit SHA** (immutable), with the human-readable version in a trailing comment. **No third-party actions** at all; the Rust toolchain is installed from the runner's pre-installed `rustup` rather than a toolchain action, removing that supply-chain edge entirely. |
| **Tampering** — Script/expression injection: untrusted `${{ github.event.* }}` data (PR title, branch name, body) interpolated into a shell. | The workflow reads attacker-controlled fields — notably the head branch name used for the `fix/`|`feat/` gate. | No `${{ github.event.* }}` value is ever interpolated into a `run:` shell. The branch gate uses `startsWith(github.head_ref, …)` inside an `if:` **expression** (evaluated by the Actions runtime, not a shell). All `run:` commands are static literals. |
| **Tampering** — Poisoned dependency or build cache restored into a trusted build. | A cache entry writable by a lower-privileged branch could inject compromised `node_modules` / Cargo artifacts. | **All caching is disabled**: `setup-node` is configured with no `cache:` key and there is no `actions/cache` step. `npm ci` installs exactly what the committed `package-lock.json` pins; the Cargo build resolves against the committed `Cargo.lock`. |
| **Repudiation** — Inability to prove which code and which action versions actually ran. | Incident response needs to reconstruct exactly what executed for a given run. | SHA-pinned actions mean the exact action source is recoverable and immutable; the toolchain is pinned to `stable` and dependencies to lockfiles; GitHub retains per-run logs bound to the triggering commit SHA. |
| **Information disclosure** — Untrusted PR code exfiltrating secrets or the token from the environment/logs. | The job builds untrusted fork code on a runner that carries a `GITHUB_TOKEN`. | The workflow runs on **`pull_request`, never `pull_request_target`**, so fork PRs get a **read-only token and no repository/organization secrets**. `permissions:` is default-deny; the job holds only `contents: read`. `persist-credentials: false` keeps even that token out of `.git/config`, so later steps and build code cannot reuse it. The workflow references **no secrets**. |
| **Denial of service** — Runner/minute exhaustion from rapid pushes or PR spam. | Each `synchronize` on an open PR would otherwise launch a fresh 2-OS matrix build. | A `concurrency` group keyed on the PR number/ref with `cancel-in-progress: true` collapses superseded runs. The `fix/`/`feat/` head-branch `if:` gate bounds which PRs run at all. `fail-fast: false` is scoped to the matrix only (so one OS failing doesn't mask the other) and does not widen the blast radius. |
| **Elevation of privilege** — An over-permissioned `GITHUB_TOKEN` letting untrusted code write to the repo (push, tags, releases, comments). | GitHub's default token grants are broad (often write) and are available to every step, including untrusted build code. | Workflow-level `permissions: {}` (default-deny) with an explicit least-privilege `contents: read` on the only job. **No** `write` scopes, no environment/deployment access, no OIDC/cloud credentials are granted. |
| **Elevation of privilege** — "Pwn request" against a self-hosted runner (untrusted PR code executing on persistent, network-adjacent infrastructure). | If CI ran on non-ephemeral self-hosted runners, fork code could pivot into internal infrastructure or persist between jobs. | Runs exclusively on **GitHub-hosted, ephemeral** `macos-latest` / `windows-latest` runners that are destroyed after each job. No self-hosted runners are exposed to this workflow. |

### Residual risk & assumptions

- **Sub-poll / TOCTOU on tags is out of scope** — SHA pinning removes tag
  mutability, but the actions' *transitive* trust (what `actions/checkout` itself
  pulls at runtime) is trusted as part of the GitHub-maintained toolchain.
- **First-run approval is an org control, not a workflow control** — For public
  forks, the repository/organization setting *"Require approval for all outside
  collaborators"* should remain enabled so a maintainer must approve before
  untrusted code runs. This workflow cannot enforce that on its own.
- **Currency of pinned SHAs** — Pinning trades automatic patch uptake for
  immutability. Enabling Dependabot for `github-actions` is recommended so the
  pinned `actions/*` SHAs are bumped (with review) as upstream releases land.
- **Build integrity depends on the lockfiles** — The no-cache posture assumes
  `package-lock.json` and `Cargo.lock` are themselves reviewed; a malicious
  dependency introduced *in the PR's own lockfile* still executes at build time.
  It does so, by design, only inside the read-only / no-secrets sandbox above.