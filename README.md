# actually

Actually, this tool forces Claude Code agents into creative thinking by pitting them against each other as contrarian strategists.  Discover unconventional approaches, and optionally implement them.

Great for brainstorming, and for rapidly implementing a suite of competing approaches for comparison.

Requires Claude Code.

## Install

```bash
cargo install actually
```

## Usage

```bash
actually "your task description"
```


[Important note on agent permissions](#permissions)

## Options

- `--headless` - Skip interactive TUI, run with tracing output
- `-n <count>` - Number of parallel strategies (default: 3). Higher values force increasingly unconventional approaches.
- `-m <model>` / `--model <model>` - Model to use for all Claude Code instances. If not specified, uses the model currently set as default in Claude Code.
- `--impl-model <model>` - Model to use specifically for implementation agents (Phase 3). Falls back to `--model` if not set.

## Strategy preview

After sequential strategizing, a TUI will appear with a preview of each contrarian strategy.  In the TUI, you can review the initial proposed strategies, edit them with your `$EDITOR`, chat with Claude Code about them, delete unfavorable strategies, add new strategies, or copy strategies to your clipboard.

| Key | Action |
|-----|--------|
| `?` | Show keymaps |
| `↑/↓` or `k/j` | Navigate |
| `Enter` | Edit strategy with `$EDITOR` |
| `t` | Chat about strategy with Claude |
| `o` | Add strategy |
| `d` | Delete strategy |
| `c` | Copy strategy to clipboard |
| `q` | Quit |

Below the strategies is a button: `>>> Accept all and begin implementation <<<`.  Selecting it will launch several Claude Code agents in parallel who will perform the implementation for each strategy.

## How it works

`actually` has three phases.  Phase 1 involves plan forming and operates sequentially, since each agent must reject the plans of the prior agents.  Phase 2 is an interactive TUI where you can review strategies, copy them to clipboard, delete bad ones, add new ones, even ask an agent about its chosen strategy.  Phase 3 involves implementing each plan, and is entirely optional.  As a brainstorming tool, Phase 1 and 2 are useful, but Phase 3 is only good if you want to compare concrete implementations of each strategy.

```
T: the given task
C1..Cn: Claude code instances
S1..Sn: the problem solving strategy proposed by the respective Cn instance

Phase 1: Strategies are devised sequentially, with each agent rejecting the strategies of the prior agents.

 1. Run C1 with T, collect S1
 2. Run C2 with T + -S1
 3. Run C3 with T - (S1 + S2)
 ...
 n. Run Cn with T - (S1 + S2 + S3 ... + Sn)

Phase 2: Strategies are implemented in parallel

 C1 implements S1
 C2 implements S2
 ...
 Cn implements Sn
```

## Behavior to expect

Generally, the first agent (`C0`) will produce the most obvious strategy.  Subsequent agents' strategies will become increasingly "out-there" as they reject the previous agents' more mainstream strategies.

Most of the time, I use `actually` purely for brainstorming, and I exit `actually` instead of selecting `>>> Accept all and begin implementation <<<`.  Implementation _can_ be interesting if you want to see multiple approaches for side-by-side comparison, but usually the strategy review phase is enough to get some novel ideas.

## Permissions

`actually` uses different permission modes per phase:

- **Phase 1 (strategizing)**: Agents run in plan mode — read-only access to `$PWD` only, web search allowed, no writes, no command execution.
- **Phase 2 (review TUI)**: No agents are active, so no permissions are needed.  The only way to launch an agent during this phase is the "Chat about strategy" feature (`t` key) which launches an interactive `claude` subprocess with your default Claude Code permissions.
- **Phase 3 (implementation)**: Entirely optional — only triggered if you explicitly select `>>> Accept all and begin implementation <<<` in the TUI. If you do, agents run with `--dangerously-skip-permissions` because approvals for a fleet of agents is overwhelming. [YOLO](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/#toc_13). The agents *could do anything*. No warranty, express or implied, etc.

## AI Disclosure

Unsurprisingly, much of `actually`'s code was produced by Claude Code.  

## License

Licensed under either MIT or Apache-2.0, at your option.
