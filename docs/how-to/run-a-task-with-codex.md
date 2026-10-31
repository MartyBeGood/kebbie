# Run a task with Codex

Run a task on OpenAI's Codex CLI instead of Claude Code.

A Codex task gets the same pooled worktree, card on the board, push block and `kebbie report` before each turn ends. The lead and the reviewer stay Claude.

Its task role is written for Codex. Codex can't run kebbie's setup and rules skills, so it prepares its worktree with the repo's own `worktree.setup_command` and is not told the rules skill. Its commits are still checked against the rules skill when it reports ready, as any task's are.

## Before you start

- Codex CLI 0.163.0 or later on `PATH`, logged in (`codex login`). `kebbie doctor` shows its version, and its `codex login` line says whether `codex login status` reads logged in.
- herdr starts it in the task's pane as `herdr agent start <task> --kind codex`.

## Install kebbie's plugin in Codex

Codex runs kebbie's hooks from a Codex plugin.

1. Install it once:

   ```bash
   kebbie setup codex
   ```

   It writes the plugin and prints one line, such as `kebbie's Codex plugin installed at ~/.codex/plugins/cache/kebbie-tasks/kebbie/local: .codex-plugin/plugin.json, hooks/hooks.json, bin/kebbie-hook`. With Codex not on `PATH` it stops and writes nothing.

2. Check it:

   ```bash
   kebbie doctor
   ```

   Its `codex plugin` line reads `ok`.

The plugin goes in Codex's plugin cache, `$CODEX_HOME/plugins/cache/kebbie-tasks/kebbie/local/` (`CODEX_HOME` defaults to `~/.codex`), as the plugin `kebbie@kebbie-tasks`, and holds three hooks:

| Hook | What it does in a kebbie task |
| --- | --- |
| `SessionStart` | Records the session id Codex picked, so a restart resumes it, and gives the session its task role as context. |
| `UserPromptSubmit` | Marks the start of a turn. |
| `Stop` | Sends the turn back once when it has no `kebbie report` yet, the same as a Claude task's stop hook. |

Outside a kebbie task's worktree each hook does nothing.

- `kebbie setup codex` never edits Codex's `config.toml` or `hooks.json`.
- Codex loads the plugin only in the sessions kebbie starts. kebbie turns it on with `-c plugins.kebbie@kebbie-tasks.enabled=true` and trusts its three hooks by their hash with `-c hooks.state=…`, for that session alone. Your own Codex sessions don't load it.
- kebbie also starts each Codex session with `-c check_for_update_on_startup=false`, so Codex's update screen never stands before the task's prompt. Your own Codex sessions still check, and updating Codex is yours to do.
- Codex trusts the hooks by their command line, not by the launcher script they run. So kebbie starts a Codex task only when every file of the plugin is exactly what it writes, and a missing, out-of-date or changed plugin stops the start with `run kebbie setup codex`.
- A second run that finds nothing to change writes nothing.
- `kebbie setup codex` is yours to run: a task cannot run it.
- Codex installs an enabled plugin again from any marketplace of the same name it finds in the session's repo, and that copy replaces kebbie's. The plugin's id, `kebbie@kebbie-tasks`, is one no Claude or Codex marketplace uses, kebbie's own repo included, and kebbie refuses a Codex task in a repo whose marketplace offers a plugin under it.

The plugin follows kebbie's updates: `kebbie update` and `kebbie setup --yes` rewrite what a new kebbie writes differently, and say `Codex plugin: updated` with the files. If `kebbie doctor` still says the plugin is out of date, run `kebbie setup codex` again: it rewrites the files that changed and prints `kebbie's Codex plugin updated:` with their names. If you want the plugin gone, run `kebbie setup codex --uninstall`.

## Write a Codex profile

1. Write `~/.config/kebbie/profiles/coder.toml`:

   ```toml
   about = "Codex on a GPT model"
   for = "a second opinion on a bug, work you want Codex to do"
   agent = "codex"
   model = "gpt-5.5"
   effort = "high"
   permission_mode = "acceptEdits"
   ```

2. Check it:

   ```bash
   kebbie profiles
   ```

   Under the profile, a `codex:` line shows the Codex binary, its version and whether kebbie's plugin is installed.

A Codex profile takes `about`, `for`, `agent`, `model`, `effort`, `role`, `skills`, `permission_mode` and `sandbox.enabled`. `model` goes to `codex --model`, `effort` to `model_reasoning_effort`. `effort` takes Codex's levels, `ultra` among them, not only Claude Code's five.

- If the profile sets no `model`, Codex runs the `model` in its own `config.toml`. `kebbie profiles` shows it on the profile's `runs:` line, as `model gpt-5.5 from Codex's config.toml`, and `kebbie doctor` names it on its `codex model` line.
- If that model is one your Codex login cannot use, such as a model a ChatGPT plan does not include, the task's first turn stops on an API error. Set `model` in the profile to one it can.
- On the board's Settings page (`,`), the Profiles section marks such a profile with `⚠`, and `m` picks a model from the list Codex offers your account.

`deny`, `allow`, `[claude]` and the other `[sandbox]` keys set Claude Code's options, which Codex doesn't read. A Codex profile that has them doesn't load, and `kebbie profiles` names the key.

## Start the task

1. Start it with the profile:

   ```bash
   kebbie start flaky-test --profile coder "Find why the checkout test fails one run in ten"
   ```

The lead can choose the profile too, by its `for`.

- If Codex asks something before it runs, kebbie answers nothing and types nothing, and the start says what Codex asks. Answer it in the task's Space, and kebbie types the task's prompt in once the keeper or an open board runs. Codex 0.163 asks:

  | Codex shows | The start says Codex asks |
  | --- | --- |
  | `Welcome to Codex` and how you want to use it, or `Signed in with your ChatGPT account`, when it finds no login in its home | `you to sign in` |
  | `Codex just got an upgrade` with `Try new model` and `Use existing model` | `which model to use, after its notice of a new one` |
  | `Update available` with `Update now`, `Skip` and `Skip until next version`, in a Codex that checks for updates anyway, as under a managed config | `whether to update it first` |
  | `Trust this folder?`, the first time Codex runs in the repo | `whether you trust its folder` |
  | `Hooks need review`, for hooks of your own that are new or changed | `whether to trust the hooks it found new or changed` |

- If you already trust the folder in Codex, that question is answered for you.
- If Codex asks one of these after the start, as the trust question after you sign in, the task waits under Needs you with what Codex asks, and the lead hears it too. kebbie looks for these questions only until Codex has taken its first prompt, so a task that later shows the same words, in code or a diff, stays where it is.

## How the sandbox and the permission mode map

With `task.sandbox` on, the default, a Codex task runs in Codex's own sandbox. Codex keeps `.git` read-only there and turns the network off, so a commit or a command that needs the network asks for approval first. So do kebbie's own commands, `kebbie tell` and `kebbie report` among them, which write kebbie's state outside the worktree and reach herdr's socket. kebbie keeps its state out of the sandbox's reach on purpose, so a task can't write it directly.

Each approval reaches you and the lead, as under [Approvals and choices](#approvals-and-choices). `p`, `Yes, and don't ask again for commands that start with …`, makes Codex stop asking for that kebbie command: Codex itself adds the rule to `~/.codex/rules/default.rules`, and every later Codex session, your own included, runs that command outside its sandbox without asking.

| Permission mode | Sandbox on | Sandbox off |
| --- | --- | --- |
| none, `default`, `acceptEdits` | `--sandbox workspace-write --ask-for-approval on-request` | Codex's own config decides |
| `plan` | `--sandbox read-only --ask-for-approval on-request` | the same |
| `dontAsk`, `bypassPermissions` | `--sandbox workspace-write --ask-for-approval never`: nothing leaves the sandbox, and what it refuses fails back to Codex | `dontAsk`: `--ask-for-approval never`; `bypassPermissions`: `--dangerously-bypass-approvals-and-sandbox` |
| `auto` | `--approve-for-me`: Codex's own reviewer model answers its requests to leave the sandbox instead of you, so a command can run outside it | `--ask-for-approval on-request` with Codex's reviewer answering |

Every command Codex runs gets kebbie's push block in its environment and `KEBBIE_ADDRESS` set to the task's address. A Codex process kebbie started counts as that task when it calls kebbie, and any other Codex counts as an agent, so neither can run `kebbie push`, `approve` or another of your commands.

## Approvals and choices

When Codex stops to ask you something mid-task, such as whether to run a command outside its sandbox, a question of its own or an MCP server's form, its terminal title starts with `[ ! ] Action Required`. kebbie reads that title, so the task waits under Needs you with `answer Codex's prompt in its Space or through the lead`, and `kebbie peers` shows it as waiting for an answer. The Codex sessions kebbie starts show Codex's default title, whatever `tui.terminal_title` in your Codex config lists.

The lead hears it once, as it hears a Claude task's prompt: the question with the command, then the options in order, with the key Codex shows for each. For example:

```text
1. Yes, proceed (y)  (focused)
2. Yes, and don't ask again for commands that start with `kebbie report` (p)
3. No, and tell Codex what to do differently (esc)
```

The lead answers only on a choice you type in its chat, with `kebbie choose <task> <number> --prompt <id>`: `<number>` is the option's number, or the key it shows in its place, as in `kebbie choose <task> y --prompt <id>`, and `<id>` is the prompt's id from kebbie's message to the lead. A view kebbie cannot read as a menu reaches the lead as its last lines on screen. You can always answer in the task's Space yourself instead.

## Messages to a Codex task

Codex has no inbox kebbie can write to, so kebbie types a message into the task's composer, only when it is empty. It counts the message as taken once the composer lets it go or the session's rollout file shows it as a user turn.

- Empty means the composer shows nothing or Codex's own placeholder, `Ask Codex to do anything`. Any other text there, faint or not, keeps kebbie from typing, so nothing is glued onto it.
- The task's first prompt waits the same way: when the new Codex's composer already holds text, the start says so, and kebbie types the prompt in once the composer is empty, while the keeper or an open board runs.

## Restarts after an update

A Codex task keeps the role and hooks it started with until it restarts, and Codex never updates a running session. So kebbie restarts a Codex task as it does a Claude one, with `codex resume <session id>` in its pane, once it is idle and out of focus: after `kebbie update`, after a newer `codex` lands on `PATH`, or when it runs without kebbie's plugin.

- The same rules hold: `restart.auto` and `restart.idle_minutes` decide when, and a restart never ends a Codex mid-turn, in the pane you are in, with text in its composer or on one of its start-up questions.
- `kebbie restart` names each Codex task with the others, as `flaky-test: restarted, as kebbie update`.
- Your own Codex sessions, outside a task, are never restarted.
- A Codex task whose profile no longer loads, or no longer runs Codex, is skipped with the profile and the error, as `skipped: its profile "coder" does not load (…)`, rather than resumed on the built-in task's Claude role. Fix the profile and the next restart takes it.

## Compressed sessions

Codex can compress a session file it hasn't written to for a week to `.jsonl.zst`. kebbie still finds such a session and resumes it, but can't read it. So it doesn't send the task's first prompt again and can't tell from it whether a typed message arrived.

## What a Codex task doesn't get

- The record of test runs that `kebbie evidence` and the reviewer read.
- The vault's tools. A Codex session gets no vault MCP server, so the lead keeps the task's note: the task's role has it send the lead the note's Problem before its first change, and its decisions and solution as it goes, with `kebbie tell lead`. Nothing holds its edits until the Problem is written.
- Resumes after an API error. An error that stops Codex's turn puts the task under Needs you with the API's message, and the lead hears it, but kebbie does not type it going again.

[Profiles](../reference/configuration.md#profiles) lists every key. [`kebbie setup codex`](../reference/commands.md#setup-and-maintenance) is in the command reference.
