# keel — design

Status: draft 1 (2026-11-20). Source of truth for keel's behaviour. Update it in the same commit
when behaviour changes.

## 1. Why

The keelstackhq tools (splice, bosun, hoist, berth, hull templates) work well one by one. Nothing
sets them up together, so the owner becomes the glue. A recent bootstrap of a client repo took 4
owner round-trips, 10+ commands from 3 CLIs and a hand-written `gh` script.

keel is a thin orchestrator. `keel up` reads one manifest, runs each tool as a subprocess, reads
its JSON, and stops at the first step only a human can do. keel never reimplements a tool.

## 2. Principles

- **Idempotent.** Each step checks first and skips what is done. A re-run resumes.
- **One answer.** `--json` prints exactly one object: done, the single human step left (with the
  exact link or command), a failed step, or an error.
- **Subprocesses only.** keel calls `gh`, `git`, `npm`, `node`, `cargo`, `splice`, `bosun` and
  `hoist`. It has no HTTP client of its own (so no TLS stack: the "rustls only" family rule holds
  trivially). GitHub reads and writes go through `gh api` / `gh secret` / `gh pr`.
- **Adapters.** Each tool has one small module (`src/tools/<tool>.rs`) that builds argv and
  parses output. When a tool lacks a feature keel needs (old version), the adapter degrades to a
  clear human step or a warning, never a crash.
- **Secrets.** The PAT is read from stdin (hidden prompt on a TTY) and written to `gh secret set`
  on its stdin. It never appears in argv, files, logs, stdout, stderr or JSON, and it is zeroized
  after use.
- `#![forbid(unsafe_code)]`. Tests never reach a real API or a real tool: they use a fake runner
  (unit) or stub scripts on `PATH` (integration).

## 3. CLI

```
keel [-C DIR] [--json] [-v] up     [--repo OWNER/NAME] [--template rust|ts|SOURCE] [--pieces ID[:NAME],...]
                                   [--plan FILE] [--wait SECS] [--token-stdin] [--confirm STEP]...
keel [-C DIR] [--json] [-v] plan   (same flags as up; writes nothing)
keel [-C DIR] [--json] [-v] doctor
```

- `-C DIR`: the repo checkout (default `.`).
- `--repo`, `--template`, `--pieces`, `--plan`: written to `keel.toml` on the first run. Later runs
  read `keel.toml`; a flag that disagrees with it is a usage error (exit 3, names the key).
- `--pieces` takes a comma list; each entry is a splice piece `ID` or `ID:NAME` (the instance name,
  e.g. `rust-cli:my-cli`).
- `--template`: `rust` = `gh:keelstackhq/hull//template-rust`, `ts` = `gh:keelstackhq/hull//template`;
  anything else is passed to `splice new` as the source.
- `--wait SECS` (default 300): how long a GitHub-App gate polls before it returns exit 2. `0` = check
  once. Poll interval: `KEEL_POLL_SECS` (default 10).
- `--token-stdin`: read the PAT for step 6 from stdin (one line, trailing newline trimmed).
- `--confirm agent-env`: record that the cloud environment setup script is attached (step 7);
  stored in `keel.toml` `[confirmed]`.
- Example: `npx @keelstackhq/keel up --repo owner/name --template rust --pieces rust-cli:my-cli`.

### 3.1 Exit codes

0 done · 1 a step failed or (for `plan`/`doctor`) work or drift remains · 2 a human step is
pending · 3 usage, manifest or environment error (message says how to fix). clap usage errors exit
3 too (help/version exit 0).

### 3.2 JSON

```json
{"version":1,"command":"up","ok":false,"exit_code":2,"status":"human",
 "steps":[{"id":"preflight","action":"ok","detail":"node 24.1.0, gh as octo, git 2.47"},
          {"id":"apps","action":"pending","detail":"Renovate app not installed on owner/name"}],
 "human_step":{"id":"apps","title":"Install the Renovate GitHub App on owner/name",
               "url":"https://github.com/apps/renovate/installations/new","command":"keel up"}}
```

- `status`: `done` | `human` | `failed` | `planned` | `error`.
- `steps[].action`: `ok` (already done, skipped) · `done` (keel did it now) · `would-run` (`plan`)
  · `pending` (human gate) · `failed` · `not-reached`.
- `human_step`: null, or `{id, title, url, command}` (`url`/`command` may be `""`). At most one.
- On exit 3: same envelope, `status:"error"`, `steps:[]`, `human_step:null`, plus
  `error:{message, fix}`.
- Without `--json`: one line per step, then the human step or `done`. Logs go to stderr.

## 4. Manifest: `keel.toml`

```toml
repo = "owner/name"
template = "rust"                 # or a splice source
pieces = ["rust-cli:my-cli"]
plan = "docs/plan.md"             # optional: bosun plan import
branch = "keel/bootstrap"         # optional; default below

[token]                           # optional; defaults shown
secret = "SPLICE_BOT_TOKEN"
environment = "splice-bot"

[confirmed]                       # written by --confirm
agent_env = true
```

Unknown keys are an error (exit 3, names the key). keel writes the file on the first run (before
any other write) and commits it with the render.

## 5. Steps of `keel up`

Each step is `check` (read-only) then, when needed, `apply`, then `check` again. `up` stops at the
first step that is pending (human) or failed; the remaining steps are `not-reached`. `plan` runs
every `check` only and marks work `would-run`; it stops after the first human gate.

| # | id | check (read-only) | apply |
|---|----|-------------------|-------|
| 1 | `preflight` | `node --version` ≥ 24, `gh auth status`, `gh api user` (login), `git --version`; repo exists (`gh repo view`); `-C` is a git checkout whose `origin` is the repo; each tool's `--version` equals its pin | install each tool not at its pin (§6) |
| 2 | `apps` | Claude and Renovate apps installed on the repo (`gh api user/installations`, then the installation's repositories when `repository_selection` is `selected`) | none: human gate; prints the install link and polls up to `--wait` |
| 3 | `render` | `.splice/lock.toml` exists and `Cargo.lock` exists when `Cargo.toml` does | branch (§5.1, switched before any commit); commit `keel.toml` alone (splice refuses a dirty tree); `splice pieces --json` validates piece names; `splice --json --yes new <source> . --force --piece …` (rust: `--data no_placeholder=true`); `cargo generate-lockfile` when needed; ensure `.keel/` is in `.gitignore`; commit (a lockfile-only re-run commits just `Cargo.lock`) |
| 4 | `governance` | `.bosun/version` exists; with `plan`, `bosun plan import FILE --dry-run` reports nothing to create | `bosun init --owner <gh login> --branch <current> --no-pr` (+ `--stack rust` for the rust template); labels fallback (§10); then `bosun plan import FILE` |
| 5 | `settings` | `hoist --json check` exits 0 | `hoist --json apply --yes`; hoist human steps become keel's human step |
| 6 | `token` | `gh api repos/R/environments/ENV/secrets/NAME` succeeds | with `--token-stdin`: read stdin, `gh secret set NAME --env ENV --repo R` (value on stdin). Without: human gate with the PAT link (§7) |
| 7 | `agent-env` | `keel.toml` `[confirmed] agent_env = true` | write `.keel/cloud-setup.sh` (§8); human gate: attach it, then `keel up --confirm agent-env` |
| 8 | `doctor` | `bosun doctor --remote` (`--all` when supported) has no `missing` item and exits 0; a PR from the branch exists | commit, `git push -u origin <branch>` (never force), `gh pr create` |
| 9 | `merge` | the PR is merged (`gh pr view --json state,headRefOid`) | none: human gate "approve head SHA `<sha>` and merge PR #N" |

### 5.1 Branch

Steps 3–8 work on one branch, resolved by `steps::work_branch`: the current branch when it is not
the default branch and HEAD is not detached, else `keel.toml` `branch` (default `keel/bootstrap`).
Render, governance and doctor call `git::ensure_branch` before they commit or push: it checks the
branch out when it exists locally, else fetches `origin/<branch>` (best effort) and tracks it when
the remote has it (a re-run from a fresh clone resumes), else creates it. It fails, so nothing is
committed or pushed, when the target is the default branch. Doctor and merge only resolve the
name (no switching): after the merge, on the default branch, they look up the PR of the manifest
branch. Commits never include `.keel/` (`git add -A -- . ':(exclude).keel'`). keel never merges,
tags, releases, publishes or force-pushes.

### 5.2 Tool exit codes

- splice: 0 ok, 1 conflict/drift → step failed, 2 usage (old splice: feature missing), 3 error.
- bosun: 0 ok, 1 error, 2 usage (feature missing), 3 conflict, 4 halted, 5 needs-human → human
  step from `error.fix` / `next`, 6 limit. JSON `{version, command, ok, exit_code, data, error, next}`.
- hoist: 0 ok, 1 drift/failed, 2 human steps (`human_steps[0]` becomes keel's), 3 error.
  JSON `{version, command, ok, exit_code, items, human_steps}`.
A non-JSON or missing output is a failed step whose detail names the command (never its stdin).

### 5.3 `keel doctor`

Runs step 1's checks (no installs) and step 8's checks alone. Exit 0 healthy, 1 drift, 3 error.

## 6. Pins and installs

Versions follow hull's `pins.toml` (hull#132): a generated repo has `pins.toml` at its root with
one `[pins]` table of exact string versions (`rust`, `nextest`, `cargo_deny`, `splice`, `bosun`,
`hoist`; keys `[a-z_][a-z0-9_]*`). keel embeds a default (`src/pins.toml`, compiled in) with the
same `[pins]` table for `splice`, `bosun`, `hoist`, `berth`, plus `[sha512.<tool>]` tables
(`version`, `launcher`, `linux_x64`, `linux_arm64`, …: npm `dist.integrity`, base64).
`KEEL_PINS=FILE` replaces the embedded file. The repo's `pins.toml` `[pins]` versions override the
embedded ones (other tables and keys are ignored). A version with no recorded sha512 (repo pins a
version keel has no sha512s for) is a failed step: update keel or point `KEEL_PINS` at a file
with that version's sha512s. keel never installs an unverified tool.

Install of a tool not at its pin: in a temp dir `npm pack @keelstackhq/<tool>@<v>` and
`npm pack @keelstackhq/<tool>-<os>-<arch>@<v>`; the base64 sha512 of each tarball must equal its
pin (mismatch: step failed, nothing installed); then
`npm install -g --ignore-scripts --omit=optional <launcher.tgz> <platform.tgz>`. A platform with
no sha512 pin (e.g. macOS, Windows) is a human step (id `preflight`) naming the manual
`npm install -g @keelstackhq/<tool>@<version>` and saying keel could not verify it.

## 7. The PAT gate

Link (fine-grained PAT, prefilled):
`https://github.com/settings/personal-access-tokens/new?name=keel%20OWNER%2FNAME&description=…&target_name=OWNER&expires_in=90&contents=write&pull_requests=write&workflows=write`
(Contents, Pull requests, Workflows: read and write; repository access: only the repo).
Command: `keel up --token-stdin` (paste, then Enter). keel stores it with
`gh secret set NAME --env ENV --repo R` reading the value on stdin.

## 8. Agent environment

`.keel/cloud-setup.sh` = a proxy-CA prelude + the `data.script` of `bosun setup-snippet`. The
prelude runs `bash scripts/proxy-ca.sh install` in each checkout that has it (hull#132's hook);
otherwise, when `/root/.ccr/ca-bundle.crt` (or `$SSL_CERT_FILE`) exists, it writes
`/etc/profile.d/keel-proxy-ca.sh` exporting `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`,
`REQUESTS_CA_BUNDLE`, `CARGO_HTTP_CAINFO`, `GIT_SSL_CAINFO` and sets `npm config set cafile`, so
tools behind the agent proxy verify TLS. The human step says where to paste it (claude.ai → Code →
environment settings → Setup script) and to run `keel up --confirm agent-env` afterwards. The file
is not committed (`.keel/` is in `.gitignore`).

## 9. npm launcher

`@keelstackhq/keel` with `optionalDependencies` on `@keelstackhq/keel-{linux-x64,linux-arm64,
darwin-x64,darwin-arm64,win32-x64}`, same launcher as hoist (`npm/keel/bin/keel.js`). Publishing
is human-only (Blocked in `docs/PROGRESS.md`).

## 10. Open questions (for the orchestrator)

Answered: pins (§6, hull#132 `pins.toml`), PAT secret `SPLICE_BOT_TOKEN` in `splice-bot`, proxy-CA
hook (§8, `scripts/proxy-ca.sh install`), labels: `bosun init` creates labels and owners from the
gh user; when its JSON has no `data.labels`, keel falls back to `gh label create … --force`. The
rust template gets `--data no_placeholder=true` (skips `crates/common`). Still assumed, behind
adapters:

1. `splice pieces --json` shape: assumed `{ok, command, pieces:[{id, description, multiple}]}`.
2. `bosun doctor --all`: assumed a flag of `doctor`; keel retries without it on exit 2.
3. `bosun plan import --dry-run` JSON: assumed `data.create` (array) — empty = done.
