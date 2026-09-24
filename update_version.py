#!/bin/python3
"""
Updates common files to a consistent version number
"""

import argparse
import re
import subprocess
from pathlib import Path


class ProgramArguments(argparse.Namespace):
    def __init__(self):
        super().__init__()
        self.version_str: str
        self.no_cargo: bool = False
        self.tag: bool = True


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
        "--no-cargo",
        action="store_true",
        help="skip running the cargo command to update the lock file",
    )
    _ = p.add_argument(
        "-t",
        "--tag",
        action="store_true",
        help="tag the current build with the given release name"
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

    should_run_cargo: bool = not args.no_cargo

    if should_run_cargo:
        p = subprocess.Popen(
            ["cargo", "test", "--workspace"],
            stdin=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=base_path,
        )
        _ = p.communicate()
        if p.returncode != 0:
            raise RuntimeError("unable to process output")

    if args.tag:
        p = subprocess.Popen(["git", "add", "."])
        _ = p.communicate()
        if p.returncode != 0:
            raise RuntimeError("unable to add new files")

        p = subprocess.Popen(["git", "commit", "-m", "Update Version"])
        _ = p.communicate()
        if p.returncode != 0:
            raise RuntimeError("unable to commit the tag values")

        p = subprocess.Popen(["git", "tag", f"v{version_str}", "-m", f"Release v{version_str}"])
        _ = p.communicate()
        if p.returncode != 0:
            raise RuntimeError("unable to set the current tag value")


if __name__ == "__main__":
    main()
