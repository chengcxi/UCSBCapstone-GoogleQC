"""Expose the exact rustfmt patch through Checks API when raw CI logs are unavailable.

Only runs in the disposable CI checkout after `cargo fmt --check` fails.
"""
import base64
import gzip
import subprocess

subprocess.run(["cargo", "fmt"], check=True)
patch = subprocess.check_output(["git", "diff", "--binary", "--"])
if not patch:
    raise SystemExit("rustfmt check failed, but formatting produced no tracked diff")
payload = base64.b64encode(gzip.compress(patch)).decode("ascii")
chunks = [payload[i:i + 1800] for i in range(0, len(payload), 1800)]
for index, chunk in enumerate(chunks, start=1):
    print(f"::notice title=rustfmt patch {index}/{len(chunks)}::{chunk}")
