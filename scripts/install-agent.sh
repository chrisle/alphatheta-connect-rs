#!/usr/bin/env bash
# Installs the porting agent and skill from this checkout's (git-ignored)
# .claude/ directory into the runner host's user-level Claude configuration,
# where the sync-upstream workflow expects them.
#
# Usage: scripts/install-agent.sh [user@host ...]   (default: agent-runner)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
agents="$root/.claude/agents"
skills="$root/.claude/skills"

if [ ! -f "$agents/upstream-porter.md" ] || [ ! -f "$skills/port-upstream/SKILL.md" ]; then
  echo "error: $root/.claude/ does not hold the upstream-porter agent and port-upstream skill." >&2
  echo "       They are git-ignored; copy them from the machine that has them." >&2
  exit 1
fi

hosts=("$@")
[ ${#hosts[@]} -eq 0 ] && hosts=(agent-runner)

for host in "${hosts[@]}"; do
  echo "==> $host"
  ssh "$host" 'mkdir -p ~/.claude/agents ~/.claude/skills'
  rsync -a --delete "$skills/port-upstream/" "$host:~/.claude/skills/port-upstream/"
  rsync -a "$agents/upstream-porter.md" "$host:~/.claude/agents/upstream-porter.md"
  ssh "$host" 'ls -la ~/.claude/agents/upstream-porter.md ~/.claude/skills/port-upstream/SKILL.md'
done
