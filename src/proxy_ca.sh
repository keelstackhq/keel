#!/usr/bin/env bash
# keel: trust the agent proxy's CA so git, npm, cargo and pip verify TLS behind it.
# Never fails the setup script.
ran=0
for d in . ./*/; do
  [ -f "$d/scripts/proxy-ca.sh" ] || continue
  (cd "$d" && bash scripts/proxy-ca.sh install) || true
  ran=1
done
if [ "$ran" = 0 ]; then
  ca=""
  if [ -f /root/.ccr/ca-bundle.crt ]; then
    ca=/root/.ccr/ca-bundle.crt
  elif [ -n "${SSL_CERT_FILE:-}" ] && [ -f "$SSL_CERT_FILE" ]; then
    ca="$SSL_CERT_FILE"
  fi
  if [ -n "$ca" ]; then
    {
      echo "export NODE_EXTRA_CA_CERTS=\"$ca\""
      echo "export SSL_CERT_FILE=\"$ca\""
      echo "export REQUESTS_CA_BUNDLE=\"$ca\""
      echo "export CARGO_HTTP_CAINFO=\"$ca\""
      echo "export GIT_SSL_CAINFO=\"$ca\""
    } > /etc/profile.d/keel-proxy-ca.sh 2>/dev/null || true
    npm config set cafile "$ca" 2>/dev/null || true
    export NODE_EXTRA_CA_CERTS="$ca" SSL_CERT_FILE="$ca" REQUESTS_CA_BUNDLE="$ca" CARGO_HTTP_CAINFO="$ca" GIT_SSL_CAINFO="$ca"
  fi
fi
true
