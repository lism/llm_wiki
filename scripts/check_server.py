"""Compare wiki directories."""
import subprocess, os, tempfile

HOST = "199.66.62.239"
USER = "root"
PASS = "admin@123"

askpass = tempfile.NamedTemporaryFile(mode="w", suffix=".sh", delete=False)
askpass.write("#!/bin/sh\necho '" + PASS + "'\n")
askpass.close()
os.chmod(askpass.name, 0o700)

cmds = """echo '=== listing ===' && find newwiki -type f 2>/dev/null | sort"""
# Just list local files instead of SSH
import os as _os
for root, dirs, files in _os.walk(r"C:\Users\idt\Downloads\llm-wiki-standalone-windows\newwiki"):
    # Skip hidden dirs
    dirs[:] = [d for d in dirs if not d.startswith('.')]
    level = root.replace(r"C:\Users\idt\Downloads\llm-wiki-standalone-windows\newwiki", "")
    for f in sorted(files):
        path = _os.path.join(level, f).lstrip("\\")
        print(f"  {path}")
