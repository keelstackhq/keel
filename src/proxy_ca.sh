#!/usr/bin/env bash
# keel: trust the agent proxy's CA so git, npm, cargo and pip verify TLS behind it.
# Never fails the setup script.
ran=0
for d in . ./*/; do
  [ -f "$d/scripts/proxy-ca.sh" ] || continue
  if (cd "$d" && bash scripts/proxy-ca.sh install); then
    ran=1
  fi
done
ca=""
if [ -f /root/.ccr/ca-bundle.crt ]; then
  ca=/root/.ccr/ca-bundle.crt
elif [ -n "${SSL_CERT_FILE:-}" ] && [ -f "$SSL_CERT_FILE" ]; then
  ca="$SSL_CERT_FILE"
fi
if [ -n "$ca" ]; then
  if [ "$ran" = 0 ]; then
    # No repo hook (or it failed): persist the CA variables ourselves.
    {
      echo "export NODE_EXTRA_CA_CERTS=\"$ca\""
      echo "export SSL_CERT_FILE=\"$ca\""
      echo "export REQUESTS_CA_BUNDLE=\"$ca\""
      echo "export CARGO_HTTP_CAINFO=\"$ca\""
      echo "export GIT_SSL_CAINFO=\"$ca\""
    } > "${KEEL_PROFILE_DIR:-/etc/profile.d}/keel-proxy-ca.sh" 2>/dev/null || true
    npm config set cafile "$ca" 2>/dev/null || true
  fi
  # The rest of the setup script (the bosun snippet) runs in this shell.
  export NODE_EXTRA_CA_CERTS="$ca" SSL_CERT_FILE="$ca" REQUESTS_CA_BUNDLE="$ca" CARGO_HTTP_CAINFO="$ca" GIT_SSL_CAINFO="$ca"
fi
true
