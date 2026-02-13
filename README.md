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

**Important note**: The Claude Code agents launched by `actually` run with `--dangerously-skip-permissions` because approvals for a fleet of agents is overwhelming.  [Besides](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/#toc_13).  Be warned: If you run `actually`, the agents *could do anything*.  No warranty, express or implied, etc.

## Options

- `--headless` - Skip interactive TUI, run with tracing output
- `-n <count>` - Number of parallel strategies (default: 3). Higher values force increasingly unconventional approaches.

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

## Behavior to expect

Generally, the first agent (`C0`) will produce the most obvious strategy.  Subsequent agents' strategies will become increasingly "out-there" as they reject the previous agents' more mainstream strategies.

Most of the time, I use `actually` purely for brainstorming, and I exit `actually` instead of selecting `>>> Accept all and begin implementation <<<`.  Implementation _can_ be interesting if you want to see multiple approaches for side-by-side comparison, but usually the strategy review phase is enough to get some novel ideas.

## AI Disclosure

Unsurprisingly, much of `actually`'s code was produced by Claude Code.  

## License

Licensed under either MIT or Apache-2.0, at your option.
