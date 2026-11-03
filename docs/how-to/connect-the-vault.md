# Connect the vault

Give every task a note in your vault, such as [mindin](https://github.com/gronare/notes-vault-mcp), and log every push.

1. Open the setup guide at its Vault step:

   ```bash
   kebbie setup vault
   ```

2. Pick a local folder and type its path, or an S3 bucket with its endpoint, bucket, access key and secret, then `⏎`.

3. At the Vault for kebbie step, pick the vault command kebbie itself runs. Each choice says where it was found and what it answered, such as `uvx mindin==0.14.0 · mindin 0.14.0`.

4. To list the vault's backlog notes on the board, tick Vault in the Backlog step (`←` goes back to it). The first source ticked is where the lead files new ideas.

5. On the last step, press `⏎`. The guide installs the vault plugin in Claude Code, gives it your answers, and writes your vault command to kebbie's `config.toml` as `vault.command`.

6. Restart the lead so it picks up the vault in its role:

   ```bash
   kebbie lead --restart
   ```

7. Check it. The `vault` line reads `ok` with the vault's name, version and capabilities.

   ```bash
   kebbie doctor
   ```

- If the Vault for kebbie step is not shown, setup found no vault command on this machine: run `kebbie setup` again once the vault plugin is installed, or set `vault.command` yourself, below.
- If the `vault` line reads `too old`, run `claude plugin update vault@gronare`, or point `vault.command` at a newer vault.
- If the `vault` line names the package's new name, run `kebbie doctor --fix` and answer `y` to write the new `vault.command`.
- If you run it without a terminal, `kebbie setup vault` shows each `claude plugin` command and runs it only on your `y`; set the vault's options in Claude Code's `/plugin` screen after.
- If a task runs in Codex or Cursor, that agent reaches the vault through its own MCP server. `kebbie setup codex` adds it to Codex. See [Each agent reaches the vault on its own](../explanation/vault.md#each-agent-reaches-the-vault-on-its-own).

## Share the vault between machines

When you run kebbie on a second machine against the same vault, give each machine its own name in its own config.toml. The other machine then sees which one works on a note.

```toml
[machine]
name = "laptop"
```

The `machine` line of `kebbie doctor` shows the name in use. What a claim does: [Two machines, one vault](../explanation/vault.md#two-machines-one-vault).

## Run the vault from your own checkout

kebbie runs the command in `vault.command`. To run another one, such as a checkout of your own, set it in `config.toml`:

```toml
[vault]
command = "uv run --project ~/code/mindin mindin"
```

`kebbie doctor` then shows what it answered. What a vault command answers: [The vault contract](../reference/vault-contract.md).

See also: [The vault](../explanation/vault.md).
