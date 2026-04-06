#!/usr/bin/env python3
"""Legacy wrapper for the default kamishibai CLI."""

from kamishibai.app import run_legacy_default


def main(argv=None):
    """Run the default-language CLI wrapper."""
    return run_legacy_default(argv)


if __name__ == "__main__":
    raise SystemExit(main())
