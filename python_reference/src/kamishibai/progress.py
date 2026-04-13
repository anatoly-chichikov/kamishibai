#!/usr/bin/env python3
"""
Observer-based progress display for card generation pipeline
"""

import os
from typing import Protocol, final


@final
class AlignedStatus:
    """Spinner wrapper that indents to align with checkmark output"""

    def __init__(self, live, spinner):
        self._live = live
        self._spinner = spinner

    def update(self, text):
        """Delegate text update to the underlying spinner"""
        self._spinner.update(text=text)

    def start(self):
        """Delegate start to the underlying live display"""
        self._live.start()

    def stop(self):
        """Delegate stop to the underlying live display"""
        self._live.stop()


class Progress(Protocol):
    """Notification API for pipeline progress events"""

    def card(self, index, total, word):
        """Signal start of a new card"""
        ...

    def step(self, name):
        """Signal start of a processing step"""
        ...

    def done(self, name, label, path=""):
        """Signal completion of a processing step"""
        ...

    def retry(self, name, attempt, reason):
        """Signal a retry within a step"""
        ...

    def skip(self, word, reason):
        """Signal that a card was skipped"""
        ...

    def result(self, label, path):
        """Signal a final output artifact with clickable path"""
        ...

    def finish(self, successful, total, failures):
        """Signal end of pipeline with summary"""
        ...


@final
class PlainProgress:
    """Plain text progress output for non-interactive terminals"""

    def __init__(self, output):
        self._output = output

    def card(self, index, total, word):
        """Print card header line"""
        self._output(f"Processing card {index}/{total}: {word}")

    def step(self, name):
        """No-op for plain mode since done() prints the result"""

    def done(self, name, label, path=""):
        """Print step completion with optional basename"""
        suffix = f" ({os.path.basename(path)})" if path else ""
        self._output(f"  {name}: {label}{suffix}")

    def retry(self, name, attempt, reason):
        """Print retry message"""
        self._output(f"  {reason} (attempt {attempt}), retrying...")

    def skip(self, word, reason):
        """Print skip message"""
        self._output(f"  Skipping {word} - {reason}")

    def result(self, label, path):
        """Print output artifact with full path"""
        self._output(
            f"  {label}: {os.path.basename(path)} ({path})"
        )

    def finish(self, successful, total, failures):
        """Print final summary"""
        self._output(f"\nProcessed {successful}/{total} cards")
        if failures:
            self._output(f"Skipped {len(failures)} card(s):")
            for item in failures:
                self._output(f"  - {item['word']}: {item['reason']}")


@final
class RichProgress:
    """Interactive terminal progress with spinners via rich"""

    def __init__(self, console, spinner):
        self._console = console
        self._spinner = spinner

    def card(self, index, total, word):
        """Print card header with rich markup"""
        self._console.print(f"[bold]{word}[/bold] ({index}/{total})")

    def step(self, name):
        """Start spinner for a step"""
        self._spinner.update(f"{name}...")
        self._spinner.start()

    def done(self, name, label, path=""):
        """Stop spinner and print checkmark with optional clickable link"""
        self._spinner.stop()
        suffix = f" ([link=file://{path}]{os.path.basename(path)}[/link])" if path else ""
        self._console.print(f"  [green]\u2714[/green] {name}: {label}{suffix}", highlight=False)

    def retry(self, name, attempt, reason):
        """Print retry inline without stopping spinner"""
        self._spinner.stop()
        self._console.print(
            f"  [yellow]\u21bb[/yellow] {reason} (attempt {attempt})"
        )
        self._spinner.start()

    def skip(self, word, reason):
        """Stop spinner and print cross mark"""
        self._spinner.stop()
        self._console.print(f"  [red]\u2718[/red] Skipping {word} - {reason}")

    def result(self, label, path):
        """Print output artifact with clickable rich link"""
        self._console.print(
            f"  [green]\u2714[/green] {label}: [link=file://{path}]{os.path.basename(path)}[/link]",
            highlight=False,
        )

    def finish(self, successful, total, failures):
        """Print final rich summary"""
        self._console.print(
            f"\n[bold]Processed {successful}/{total} cards[/bold]"
        )
        if failures:
            self._console.print(
                f"[yellow]Skipped {len(failures)} card(s):[/yellow]"
            )
            for item in failures:
                self._console.print(
                    f"  - {item['word']}: {item['reason']}"
                )


@final
class ProgressSelector:
    """Selects appropriate Progress implementation based on terminal capability"""

    def __init__(self, terminal):
        self._terminal = terminal

    def selected(self):
        """Return RichProgress if interactive terminal, PlainProgress otherwise"""
        if self._terminal:
            from rich.console import Console
            from rich.live import Live
            from rich.padding import Padding
            from rich.spinner import Spinner
            console = Console()
            spinner = Spinner("dots")
            padded = Padding(spinner, pad=(0, 0, 0, 2), expand=False)
            live = Live(padded, console=console, transient=True, refresh_per_second=12.5)
            return RichProgress(console, AlignedStatus(live, spinner))
        return PlainProgress(print)
