#!/usr/bin/env python3
"""Explicit short-clip research entry point for the production ACP transport."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from antigravity_audio import main
if __name__ == "__main__":
    main()
