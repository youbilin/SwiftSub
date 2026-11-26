"""Validate generated 100-node presets with a caller-supplied Mihomo binary."""
import argparse
import pathlib
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mihomo", required=True)
    parser.add_argument("--data-dir", required=True)
    parser.add_argument("--profiles", required=True)
    args = parser.parse_args()
    binary = pathlib.Path(args.mihomo).resolve(strict=True)
    directory = pathlib.Path(args.data_dir).resolve(strict=True)
    profiles = pathlib.Path(args.profiles).resolve(strict=True)
    presets = ["minimal", "standard", "full"]
    presets.extend(name for name in ("workspace-upgraded", "workspace-dormant", "workspace-edited-rules", "workspace-complex-rules", "workspace-renamed-ordered", "workspace-renamed-dormant") if (profiles / f"{name}.yaml").exists())
    for preset in presets:
        profile = (profiles / f"{preset}.yaml").resolve(strict=True)
        result = subprocess.run(
            [str(binary), "-t", "-d", str(directory), "-f", str(profile)],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60,
        )
        print(f"{preset}: {'PASS' if result.returncode == 0 else 'FAIL'}")
        if result.returncode:
            print(result.stdout)
            print(result.stderr)
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
