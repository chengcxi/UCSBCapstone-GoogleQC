"""Expose actionable Cargo errors in GitHub check annotations (not just the log).

The Checks API truncates a single annotation at about 4 KiB; taking the tail of
Cargo's full log loses the error if later dependencies print many warnings.
"""

import re
import sys
from pathlib import Path


def main() -> None:
    title, path = sys.argv[1:]
    text = Path(path).read_text(errors="replace")
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    lines = text.splitlines()
    beginnings = [
        index
        for index, line in enumerate(lines)
        if line.startswith(("error:", "error[", "failures:", "test result: FAILED"))
        or "panicked at" in line
    ]
    if beginnings:
        # Include the source span and the compiler's help/note beneath each error.
        body = "\n\n".join("\n".join(lines[index : index + 17]) for index in beginnings)
        body = body[:2900] + "\n\nLast output:\n" + "\n".join(lines[-12:])[-850:]
    else:
        body = "\n".join(lines[-45:])
    # Escape workflow-command characters *before* newline conversion. Limit to
    # less than GitHub's annotation size so both the error and tail remain visible.
    escaped = body[:3800].replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::error title={title}::{escaped}")


if __name__ == "__main__":
    main()
