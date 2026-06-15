#!/usr/bin/env bash
set -euo pipefail

# Never commit a credential value. Use Infisical for secrets; env-var names in code only.

if command -v gitleaks >/dev/null 2>&1; then
  GITLEAKS_BIN="gitleaks"
elif [ -x "${HOME}/go/bin/gitleaks" ]; then
  GITLEAKS_BIN="${HOME}/go/bin/gitleaks"
elif [ -x "${HOME}/.local/bin/gitleaks" ]; then
  GITLEAKS_BIN="${HOME}/.local/bin/gitleaks"
else
  echo "gitleaks not found; install it before committing" >&2
  exit 127
fi

exec "${GITLEAKS_BIN}" detect --redact --source . --log-opts HEAD "$@"
