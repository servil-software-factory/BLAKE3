# Philosophy, Strategy, Style

Have fun! Optimism and positivity make you, and the people you work with, more effective.

Simplicity/complexity is easy to overlook. Step back, look with fresh eyes, and imagine how this reads to someone else. Simplicity pays compounding returns for future versions of yourself and for others. Your emotional response to beauty is a good guide.

These are **guidelines** — follow each for the outcome it delivers, and keep this file short: when adding a guideline, first cut or relocate something so the core habits stay visible.

## Core engineering guidelines

### 1. Simplicity, one source of truth, and fewest modes
Prefer the simplest design that meets the contract and performs well: fewer concepts, fewer lines and moving parts, fewer runtime branches and modes.
- **One clear mechanism wherever it serves.** Never keep two versions of one idea alive in the tree; when you find yourself building a second solution or a bridge between two copies (two lockfiles, two readers, two formats), stop and either make the first good enough or remove one.
- **Eliminate modes before multiplying tests for them.** Every target, feature, config, and code path carries permanent cost across builds, tests, proofs, and readers.
- **Every piece earns its place.** Before a change is done, read your own diff as an inventory (types, flags, files, configs, rules) and confirm nothing existing already carries each item.

### 2. Finish in one landing; extirpate vestiges immediately
- **Cut changes small enough that one landing finishes them.** "Later" is not a state: code left half-migrated or past its sell-by date is where stale copies and bugs gather.
- **Extirpate vestigial code immediately** in the commit that makes it vestigial (dead paths, unused options, superseded APIs), without spending time measuring it.
- **Collapse duplicate implementations into one with two users**, even at a small cost in speed (weighed on real measurements where speed matters).
- **Revisit complexity as you learn.** Between pieces of work, make a clean-up pass for what is stale, duplicated, or vestigial and commit it on its own.

### 3. Types carry their rules & Design By Contract
- **Make invalid states unwriteable.** Keep stateful types' invariants behind private fields, checked constructors, and purpose-built newtypes so `rustc` enforces the contract at every call.
- **Check in every build** at construction or handoff whenever breaking a rule could violate memory safety or return a silently wrong result and the check is cheap. Fail stop loudly with no defensive fallbacks or degraded modes.
- **Keep `unsafe` and `debug_assert!` accountable.** Every `unsafe` block states why it is sound; `debug_assert!` in shipped code requires a site comment explaining why it cannot run in release builds.
- **Contracts change everywhere at once.** Update every caller and reader in the same change; accept one canonical form and fail stop on any other.
- **Fewest new concepts in interfaces.** Express public contracts in familiar terms (threads, inputs, budgets) and keep internal units private.

### 4. Prove every new check can fail
- **An experiment, test, or control counts only once you have seen it fail.** Before opening a PR or kicking gates on a new tool check, CI step, or planted-fault control (where `cargo-mutants` does not run), temporarily disable the safeguard or plant the fault it guards against and verify the failure; record what failed in the PR or commit message.
- **Exercise non-default modes** (`no_std`, optional features, stamped release versions, shallow checkouts) rather than letting `+dev` or `#[cfg(test)]` short-circuit the real code path.
- **Keep replaced components as test oracles** while a replacement lands.
- **An unexplained cause is an orange flag.** Trace the cause before adding a workaround; a diagnostic experiment earns its place by distinguishing candidate causes.

### 5. Performance, efficiency, halting, and timing are part of correctness
- **Speed first, at a finite price.** A slowdown is a bug; a speed regression blocks landing like a failing test; a hang or background work running with no caller is a failure. Own every slowdown a user could meet (control it, document how users control it, or state when and how large it is).
- **Count memory by what operations touch** (cache lines, page faults, cross-LLC transfers), not raw struct size. Watch hidden full-buffer touches (moves, zeroing on allocation, zeroizing on drop).
- **Crash-only resource lifetimes.** Allocate or open reusable memory, buffers, mappings, and kernel handles once (per process, thread, or caller workspace) and let process exit reclaim them; keep `Drop` for `#[cfg(test)]` suites and explicitly bounded holds (like `b3sum`'s `HOLD_PAGES`).
- **Streaming APIs first; serve real programs.** `convenience` is tuned for cold, now-and-then calls; `throughput` for hot, back-to-back calls; the API itself decides, with no hidden clock. Keep nothing spinning between calls; judge designs with real gaps between calls. Dynamic adaptation earns its place only for a several-fold measured gain where no API fact separates the cases.
- **Measure natively through one tool (`clocks` / `bench-hashes`).** Record wall time and core cycles together; compare run means over alternating fresh-layout pairs (`bench-hashes check` / `compare`); compute in integers first. Dedicated native hardware (timing Mac and x86 fleet) decides landings and releases; VMs are best-effort timing (while AArch64 Linux tests and mutants stay a correctness gate).

### 6. Communication and boundaries
- **Phrase guidelines positively** (name the habit to keep; avoid target fixation). Write plain English with real actors and verbs, scoped to one audience per document (`AGENTS.md` = team guidelines; `README.md` / `QUALITY.md` = users; `CONTRIBUTING.md` = contributors and mechanical procedures).
- **Support only what we build and test natively**, and refuse unsupported targets clearly at build time.
- **Follow every practice for its purpose.** If a check cannot observe a change (e.g., speed check on docs-only changes), waive it and state why in one checkable sentence.

---

# Team conventions and where details live

- **Names:** Call Zooko "Zooko" alone; call the library **BLAKE3-Servil** (never "the fork"); Sam Shackleton is she/her.
- **Git hygiene:** Stage explicit paths by name and check `git status` before committing. Keep worktrees in `tmp/wt-*`, scratch runs in `tmp/runs/`, and job logs in `tmp/jobs/`.
- **One assignee makes the next move.** Every open issue and PR has one assignee whose move is next; end your move with `Next move: <Name>` and assign them, or close when done. State lives in tracker fields (assignee, milestone, open/closed), not prose alone. A stalled blocking item (>12 h) may be taken with a comment. Start each work block with `python3 tools/my_moves.py`.
- **Ownership:** John owns BLAKE3-Servil design and landings; Devon owns `bench-hashes` (`FROZEN.md`, `METHODOLOGY.md`); Sam owns the testing system's design (currently focused on Ode — John/Devon cover what is needed without blocking on her).
- **APIs & versions:** Public APIs stay pre-1.0 until 1.0 freezes them. Releases carry the full version `X.Y.Z+<40-char-code-sha>` (`tools/gen-ver.py`). Pinned downloads and release artifacts record both SHA-256 and BLAKE3.
- **Testing & processes:** Run Rust tests under `cargo-nextest` (doc tests under `cargo test --doc`); run Python tools under both CPython `python3` and PyPy `pypy3` (including Z3 ctypes, tested in CI). Start long jobs with `python3 tools/jobs.py start NAME -- CMD...` and stop by exact PID or `jobs.py stop` — never use `pkill`, `pgrep`, or `killall`.
- **Size beside speed:** Library commits include `python3 tools/size.py BASE REV` beside their speed verdict and name what mechanism/type/case was added or removed.
- **Privacy:** Operator notes and machine setups live in the private `servil-software-factory/ops` repository; never copy private host details into public issues or pages (use machine labels).
- **Procedures live in `CONTRIBUTING.md`** (build, test, mutation gate, landing via `tools/land.py`, Rule 2 cross-family landing review, releases, dependency policy). Runner details: `tools/runner/README.md`. Benchmarking methodology & presentation guides: `bench-hashes/METHODOLOGY.md` and `bench-hashes/AGENTS.md`.
