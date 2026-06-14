#!/bin/bash
# One-time: create a long-lived self-signed code-signing certificate
# "Otto Dev Signing" in the login keychain. Signing every build with the SAME
# cert keeps the code-signature identity stable, so macOS TCC approvals
# (network, accessibility, etc.) persist across rebuilds on this machine.
set -euo pipefail

CERT_NAME="Otto Dev Signing"

if security find-certificate -c "$CERT_NAME" >/dev/null 2>&1; then
    echo "Certificate '$CERT_NAME' already exists — nothing to do."
    exit 0
fi

TMPDIR_CERT=$(mktemp -d)
trap 'rm -rf "$TMPDIR_CERT"' EXIT

cat > "$TMPDIR_CERT/cert.conf" <<EOF
[ req ]
distinguished_name = dn
x509_extensions = ext
prompt = no
[ dn ]
CN = $CERT_NAME
[ ext ]
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:false
EOF

openssl req -x509 -newkey rsa:2048 -keyout "$TMPDIR_CERT/key.pem" \
    -out "$TMPDIR_CERT/cert.pem" -days 3650 -nodes \
    -config "$TMPDIR_CERT/cert.conf"

openssl pkcs12 -export -inkey "$TMPDIR_CERT/key.pem" -in "$TMPDIR_CERT/cert.pem" \
    -out "$TMPDIR_CERT/cert.p12" -passout pass:otto

security import "$TMPDIR_CERT/cert.p12" -k ~/Library/Keychains/login.keychain-db \
    -P otto -T /usr/bin/codesign

# Trust the cert for code signing (will prompt for the login password once).
security add-trusted-cert -d -r trustRoot -p codeSign \
    -k ~/Library/Keychains/login.keychain-db "$TMPDIR_CERT/cert.pem" || {
    echo "NOTE: add-trusted-cert needs an admin prompt. If it failed, open"
    echo "Keychain Access → login → '$CERT_NAME' → Trust → Code Signing: Always Trust."
}

echo "Created '$CERT_NAME' (valid 10 years). Verify with:"
echo "  security find-identity -p codesigning -v"
