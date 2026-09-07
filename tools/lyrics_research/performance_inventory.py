#!/usr/bin/env python3
"""Research entry point for exact local spelling of performed lyric phrases."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from local_lyric_spelling import project, main
if __name__ == '__main__':
    main()
