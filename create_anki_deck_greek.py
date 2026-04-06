#!/usr/bin/env python3
"""Legacy wrapper for the Greek kamishibai CLI."""

from kamishibai.app import run_legacy_greek


def main(argv=None):
    """Run the Greek-language CLI wrapper."""
    return run_legacy_greek(argv)


if __name__ == "__main__":
    raise SystemExit(main())
