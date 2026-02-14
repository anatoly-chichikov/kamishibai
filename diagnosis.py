#!/usr/bin/env python3
"""
Error display for startup and input validation failures
"""

import sys
from typing import Protocol, final


class Display(Protocol):
    """Renders a user-facing error message"""

    def show(self, message, path):
        """Display error message with optional file path context"""
        ...


@final
class PlainDiagnosis:
    """Plain text error output for non-interactive terminals"""

    def __init__(self, output):
        self._output = output

    def show(self, message, path):
        """Print 'Error: {message}' and optional path to output"""
        self._output(f"Error: {message}")
        if path:
            self._output(f"  File: {path}")


@final
class RichDiagnosis:
    """Rich panel error output for interactive terminals"""

    def __init__(self, console):
        self._console = console

    def show(self, message, path):
        """Render a red-bordered panel with error message and optional path link"""
        from rich.panel import Panel
        body = message
        if path:
            body += f"\n\n[dim]File:[/dim] [link=file://{path}]{path}[/link]"
        self._console.print(
            Panel(body, title="Error", border_style="red", expand=False)
        )


@final
class DiagnosisSelector:
    """Selects appropriate Display implementation based on terminal capability"""

    def __init__(self, terminal):
        self._terminal = terminal

    def selected(self):
        """Return RichDiagnosis if interactive terminal, PlainDiagnosis otherwise"""
        if self._terminal:
            from rich.console import Console
            return RichDiagnosis(Console(stderr=True))
        return PlainDiagnosis(lambda text: print(text, file=sys.stderr))
