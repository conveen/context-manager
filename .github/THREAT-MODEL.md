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
  prefixed `fix/`, `feat/`, or `dependabot/` (every branch Dependabot creates,
  across all ecosystems configured in [`dependabot.yml`](./dependabot.yml));
  `workflow_call`; `workflow_dispatch`.
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
| **Tampering** — Script/expression injection: untrusted `${{ github.event.* }}` data (PR title, branch name, body) interpolated into a shell. | The workflow reads attacker-controlled fields — notably the head branch name used for the `fix/`/`feat/`/`dependabot/` gate. | No `${{ github.event.* }}` value is ever interpolated into a `run:` shell. The branch gate uses `startsWith(github.head_ref, …)` inside an `if:` **expression** (evaluated by the Actions runtime, not a shell). All `run:` commands are static literals. |
| **Tampering** — Poisoned dependency or build cache restored into a trusted build. | A cache entry writable by a lower-privileged branch could inject compromised `node_modules` / Cargo artifacts. | **All caching is disabled**: `setup-node` is configured with no `cache:` key and there is no `actions/cache` step. `npm ci` installs exactly what the committed `package-lock.json` pins; the Cargo build resolves against the committed `Cargo.lock`. |
| **Repudiation** — Inability to prove which code and which action versions actually ran. | Incident response needs to reconstruct exactly what executed for a given run. | SHA-pinned actions mean the exact action source is recoverable and immutable; the toolchain is pinned to `stable` and dependencies to lockfiles; GitHub retains per-run logs bound to the triggering commit SHA. |
| **Information disclosure** — Untrusted PR code exfiltrating secrets or the token from the environment/logs. | The job builds untrusted fork code (or a Dependabot-authored manifest/lockfile change) on a runner that carries a `GITHUB_TOKEN`. | The workflow runs on **`pull_request`, never `pull_request_target`**, so fork PRs get a **read-only token and no repository/organization secrets**. GitHub applies the same read-only/no-secrets restriction automatically for any PR authored by `dependabot[bot]`, regardless of fork status. `permissions:` is default-deny; the job holds only `contents: read`. `persist-credentials: false` keeps even that token out of `.git/config`, so later steps and build code cannot reuse it. The workflow references **no secrets**. |
| **Denial of service** — Runner/minute exhaustion from rapid pushes or PR spam. | Each `synchronize` on an open PR would otherwise launch a fresh 2-OS matrix build; Dependabot can open several PRs at once across ecosystems. | A `concurrency` group keyed on the PR number/ref with `cancel-in-progress: true` collapses superseded runs. The `fix/`/`feat/`/`dependabot/` head-branch `if:` gate bounds which PRs run at all. `fail-fast: false` is scoped to the matrix only (so one OS failing doesn't mask the other) and does not widen the blast radius. |
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
  immutability. [`dependabot.yml`](./dependabot.yml) covers the `github-actions`
  ecosystem so the pinned `actions/*` SHAs are bumped (with a PR to review) as
  upstream releases land — see the `dependabot-auto-merge` section below for how
  those PRs are handled.
- **Build integrity depends on the lockfiles** — The no-cache posture assumes
  `package-lock.json` and `Cargo.lock` are themselves reviewed; a malicious
  dependency introduced *in the PR's own lockfile* still executes at build time.
  It does so, by design, only inside the read-only / no-secrets sandbox above.

---

## Workflow: `dependabot-auto-merge`

**What it is.** [`dependabot-auto-merge.yml`](./workflows/dependabot-auto-merge.yml)
enables GitHub's native auto-merge on a Dependabot pull request once it is
opened/updated, for updates classified as low-risk. It never builds or tests
anything itself — GitHub only completes the merge once `run-ci` (or whatever
is configured as a required status check) subsequently reports success.

**Trigger & privilege surface.**
- Trigger: `pull_request_target`, scoped to base branch `master`.
- Job gate: `if: github.actor == 'dependabot[bot]'` — `actor` is set by GitHub
  from the authenticated event source, not from PR-controlled content, so it
  cannot be spoofed by opening a PR with a similar branch name or title.
- Token: default-deny (`permissions: {}`) at the workflow level; the job is
  granted `pull-requests: write` and `contents: write` — the minimum `gh pr
  merge` requires.
- Code executed: **none from the PR.** There is no `actions/checkout` step;
  the job only calls `dependabot/fetch-metadata` (reads PR metadata via the
  GitHub API) and `gh pr merge` (a GitHub API call). This is what makes the
  elevated `pull_request_target` token safe to use here — the classic "pwn
  request" risk requires checking out and executing untrusted PR code, which
  never happens in this workflow.

**Trust boundary.** The boundary is *"a Dependabot PR" vs. "a merge decision
with write access."* The design goal is that nothing other than a genuine
Dependabot PR can reach the merge step, and that even a genuine Dependabot PR
only auto-merges when it is both classified as low-risk **and** has passed CI.

| STRIDE — Threat | How it applies to `dependabot-auto-merge` | Mitigation |
|---|---|---|
| **Spoofing** — A non-Dependabot PR impersonating Dependabot (matching branch-naming conventions, PR title, or commit message format) to reach the merge step. | This is the only workflow in the repo with write access; anything that can reach it and get `gh pr merge` to run has some ability to land code without further human review. | The gate is `github.actor == 'dependabot[bot]'`, a GitHub-attested field derived from the authenticated identity that created the event — not from any PR-supplied string (title, branch name, commit trailer). A third party cannot cause GitHub to attribute a PR's actor as `dependabot[bot]`. |
| **Tampering** — Forging the "this is a security fix" / "this is patch-level" signal to get an otherwise-ineligible (e.g. major, non-security) update auto-merged. | The merge-eligibility `if:` reads `update-type` and `ghsa-id` from `dependabot/fetch-metadata`. | These outputs are derived by the official Dependabot action from Dependabot's own structured commit/PR metadata and (for `ghsa-id`/`alert-state`) a live lookup against GitHub's security-advisory data (`alert-lookup: true`), not from arbitrary PR body text an attacker controls. Combined with the actor gate above, a non-Dependabot PR cannot reach this check at all. |
| **Repudiation** — Inability to show why a given PR was or wasn't auto-merged. | Reviewers need to reconstruct, after the fact, which updates auto-merged and on what basis. | The `update-type`/`ghsa-id` eligibility check is a visible step condition in the run log; GitHub's auto-merge UI on the PR itself records that auto-merge was enabled and by which run; the actual merge commit is attributed to the `github-actions` app, distinguishing it from a human merge. |
| **Information disclosure** — Leaking the elevated `pull_request_target` token or secrets to untrusted code. | `pull_request_target` grants a normal (non-read-only) token, which would be dangerous if untrusted PR code ran with it. | No `actions/checkout` and no execution of any PR-supplied code — the job only calls two APIs (metadata fetch, PR merge). The workflow references **no repository secrets**, only the ephemeral `github.token`. |
| **Denial of service** — Dependabot opening many PRs at once, each spinning up a job. | Multiple ecosystems in [`dependabot.yml`](./dependabot.yml) can each open PRs in the same window. | Each run is a single short-lived `ubuntu-latest` job with no matrix and no build step; cost/time per run is minimal. `run-ci`'s own concurrency/branch gating bounds the more expensive matrix builds these PRs trigger. |
| **Elevation of privilege** — Auto-merging a change that introduces a regression or breaking change without human review. | Blanket auto-merge of *any* Dependabot PR would let a major version bump (or a compromised release that still happens to pass CI) land unattended. | Eligibility is restricted to `semver-patch`/`semver-minor` updates, plus `semver-major` **only** when it is itself a security fix (`ghsa-id` non-empty). A non-security major bump always requires a manual merge. This is a deliberate, narrower risk acceptance than "all updates" — see residual risk below. |
| **Elevation of privilege** — Merge completing before CI actually passes. | `gh pr merge --auto` only *enables* GitHub's native auto-merge; whether it waits for `run-ci` depends entirely on branch protection configuration, which lives outside this workflow file. | **Operational dependency, not enforced by this workflow file** — see residual risk below. |

### Residual risk & assumptions

- **Branch protection must name `run-ci` as a required status check on
  `master`.** This workflow only *enables* auto-merge; GitHub defers the actual
  merge until required status checks pass. If no required status check is
  configured, auto-merge can complete as soon as the PR is otherwise mergeable,
  **without waiting for `run-ci`.** This is a repository setting (Settings →
  Branches → branch protection rule), not something expressible in a workflow
  file, and must be configured before this workflow's "auto-merge if CI passes"
  property actually holds.
- **Security-major carve-out trusts Dependabot's advisory classification.** A
  major update auto-merges when `ghsa-id` is non-empty, i.e. when Dependabot
  itself classifies the PR as fixing a known advisory. That classification —
  and the advisory data behind it — is trusted as-is; this workflow does not
  independently verify the CVE/GHSA.
- **A security-classified major update can still contain breaking API changes**
  in addition to the fix. The auto-merge decision here accepts that risk
  deliberately, in exchange for not delaying security fixes behind manual
  review.
- **`gh pr merge --auto` does not itself re-verify `github.actor`** at merge
  completion time; the actor check happens only at the moment this workflow
  runs (PR open/synchronize). This is consistent with Dependabot PRs, which
  cannot be taken over by another author without changing the PR's identity.