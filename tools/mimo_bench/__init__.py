"""Offline-by-default benchmark harness for MiMo v2.5 audio description.

Nothing in this package opens a socket unless the driver is invoked with
``--live`` *and* the confirmation environment variable is set. Every other
entry point — planning, cost projection, request construction, scoring — is
pure and runs against stored responses.
"""

from __future__ import annotations

HARNESS_VERSION = "musializer.mimo-bench/v1"
