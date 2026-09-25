#!/bin/python3
"""
Updates common files to a consistent version number
"""

import argparse
import os
import re
import subprocess
from pathlib import Path


class ProgramArguments(argparse.Namespace):
    def __init__(self):
        super().__init__()
        self.version_str: str
        self.run_test: bool = True
        self.tag: bool = True


def run_cmd(args: list[str], cwd: os.PathLike | None = None):
    p = subprocess.Popen(
        args,
        stdin=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=cwd,
    )
    _, se = p.communicate()
    if p.returncode != 0:
        raise RuntimeError(f"unable to run '{' '.join([f'"{s}"' for s in args])}' - {se.decode('utf-8')}")


def main():
    p = argparse.ArgumentParser(
        usage="Provide a version number to update all relevant versions to the current, and update the Cargo.lock file"
    )
    _ = p.add_argument(
        "version_str",
        type=str,
        help="The version string to update versions to. Must be of the form 0.0.0",
    )
    _ = p.add_argument(
        "-n",
        "--no-test",
        dest="run_test",
        action="store_false",
        help="skip running the cargo command to update the lock file",
    )
    _ = p.add_argument(
        "-t",
        "--tag",
        action="store_true",
        help="tag the current build with the given release name",
    )

    nsp = ProgramArguments()
    args = p.parse_args(namespace=nsp)

    version_str: str = args.version_str

    version_re = re.compile(r"^\d+\.\d+\.\d+$")
    if not version_re.match(version_str):
        raise ValueError("invalid version string provided")

    base_path = Path(__file__).parent

    files_to_check: list[tuple[Path, str, str]] = [
        (
            base_path / "doc" / "isa.tex",
            r"\\date{(?P<current>v[\d\w\.]+)\s+(?P<rest>[^\s].*)}",
            rf"\\date{{v{version_str} \g<rest>}}",
        ),
        (
            base_path / "Cargo.toml",
            r"\[workspace.package\]\nversion = \"[\d\.]+\"",
            f'[workspace.package]\nversion = "{version_str}"',
        ),
        (
            base_path / "CMakeLists.txt",
            r"project\(SolariumProcessor VERSION (?P<current>[\d\.]+)\)",
            f"project(SolariumProcessor VERSION {version_str})",
        ),
    ]

    for file_path, re_str, replace_val in files_to_check:
        data = file_path.read_text()
        data = re.sub(re_str, replace_val, data)
        _ = file_path.write_text(data)

    should_run_cargo_test: bool = not args.run_test

    run_cmd(
        ["cargo", "update", "--offline"],
        cwd=base_path,
    )

    if should_run_cargo_test:
        run_cmd(
            ["cargo", "test", "--workspace"],
            cwd=base_path,
        )

    if args.tag:
        run_cmd(["git", "add", "."])
        run_cmd(["git", "commit", "-m", "Update Version"])
        run_cmd(["git", "tag", f"v{version_str}", "-m", f"Release v{version_str}"])


if __name__ == "__main__":
    main()
