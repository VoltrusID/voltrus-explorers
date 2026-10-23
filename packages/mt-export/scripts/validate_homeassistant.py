"""Validate exported Home Assistant YAML with Home Assistant's own modbus schema.

    python -m venv /tmp/hav && /tmp/hav/bin/pip install homeassistant pymodbus
    /tmp/hav/bin/python scripts/validate_homeassistant.py tests/golden/*.homeassistant.yaml
"""
import sys

import yaml

from homeassistant.components.modbus import CONFIG_SCHEMA, DOMAIN


def main(paths: list[str]) -> int:
    failed = 0
    for path in paths:
        with open(path, encoding="utf-8") as f:
            doc = yaml.safe_load(f)
        try:
            CONFIG_SCHEMA({DOMAIN: doc[DOMAIN]})
            print(f"ok    {path}")
        except Exception as err:  # noqa: BLE001
            failed += 1
            print(f"FAIL  {path}: {err}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
