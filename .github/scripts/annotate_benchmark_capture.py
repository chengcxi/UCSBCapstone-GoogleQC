"""Expose a measured CI report through Checks API when artifact downloads fail.

This is transport, not a benchmark: the preceding release binary command does
all measurement. The annotation can be compared byte-for-byte to the uploaded
artifact after base64+gzip decoding. Never print invented timing values.
"""

import base64
import gzip
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = path.read_bytes()
if not 0 < len(report) <= 120_000:
    raise SystemExit("benchmark report is empty or too large for CI annotations")
payload = base64.b64encode(gzip.compress(report)).decode("ascii")
chunks = [payload[i:i + 1800] for i in range(0, len(payload), 1800)]
for index, chunk in enumerate(chunks, start=1):
    print(f"::notice title=measured seam CPU report {index}/{len(chunks)}::{chunk}")
