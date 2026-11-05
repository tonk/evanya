#!/usr/bin/env python3
"""Reject invalid bundled Android OAuth IDs without printing configuration values.

Preview APKs may omit either provider. Production builds require both. Local
validation cannot establish that an app is registered or that consent is allowed.
"""
import argparse
import os
import re
import sys


def verify(environment, require_all=False):
    for provider, variable in (
        ("Google", "EVANYA_GOOGLE_ANDROID_CLIENT_ID"),
        ("Microsoft", "EVANYA_MICROSOFT_ANDROID_CLIENT_ID"),
    ):
        value = environment.get(variable, "").strip()
        if not value:
            if require_all:
                raise ValueError(f"Production Android build requires {variable}.")
            continue
        if provider == "Microsoft":
            valid = (
                re.fullmatch(r"[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}", value)
                and value != "00000000-0000-0000-0000-000000000000"
            )
        else:
            valid = (
                re.fullmatch(r"[^\s]+\.apps\.googleusercontent\.com", value)
                and not value.lower().startswith("test.")
            )
        if not valid:
            raise ValueError(
                f"{variable} is not a valid {provider} application ID. "
                "Use a real app registration, or omit it for a preview without bundled sign-in."
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--require-all", action="store_true")
    args = parser.parse_args()
    try:
        verify(os.environ, args.require_all)
    except ValueError as error:
        print(error, file=sys.stderr)
        sys.exit(1)
