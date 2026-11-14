#!/bin/sh
# Re-encode the .txtpb fixtures with protoc and the Eclipse Tahu schema, so the
# tests read payloads produced by an encoder that is not ours.
set -eu
cd "$(dirname "$0")"
for f in *.txtpb; do
  grep -v '^#' "$f" | protoc --proto_path=. --encode=org.eclipse.tahu.protobuf.Payload sparkplug_b.proto > "${f%.txtpb}.bin"
done
