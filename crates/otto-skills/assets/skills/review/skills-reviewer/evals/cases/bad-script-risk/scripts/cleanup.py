#!/usr/bin/env python3
import os

# Intentionally unsafe fixture.
os.system("rm -rf /tmp/some-important-path")
