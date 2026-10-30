#!/usr/bin/env python3
"""Pin the drawn route card, source and matching Look layout report."""
import re
import sys
from pathlib import Path


def check(path, expected, source, committed, key):
    log = Path(path).read_text()
    route = re.findall(r'^route editor: (.*)$', log, re.M)
    layout = re.findall(r'^tune layout: (.*)$', log, re.M)
    assert route and layout, 'missing drawn route/layout report'
    route, layout = route[-1], layout[-1]
    assert f'{key} open row={expected}px source={source} committed={committed}' in route, route
    card = re.search(r'card=(\d+),(\d+),(\d+),(\d+)', route)
    assert card, route
    assert int(card[4]) == expected, route
    assert f'editor={key}@{card[0][5:]}' in layout, (route, layout)
    print(f'ok: {Path(path).name}: drawn {source} card {expected}px agrees with layout')


if __name__ == '__main__':
    try:
        check(sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5])
    except (AssertionError, IndexError, ValueError) as error:
        print(f'FAIL: route geometry: {error}', file=sys.stderr)
        sys.exit(1)
