"""옛 형식(git-ai 1.3.x) 노트가 든 번들마다, 지금 git-ai의 `blame --json`으로 기대값을 쓴다.

사용법: python3 -I v1_expected.py <git-ai 실행 파일> <번들 디렉터리>
번들 디렉터리에는 plx_golden_v1이 쓴 `v1_<이름>.bundle`과 `v1_<이름>.meta.json`이 있다.
"""

import json
import os
import pathlib
import subprocess
import sys
import tempfile


def run(args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def main():
    git_ai = sys.argv[1]
    out = pathlib.Path(sys.argv[2])
    # "1.7.6" 또는 "1.7.6 (debug)"
    version = subprocess.run([git_ai, "--version"], check=True, capture_output=True, text=True).stdout.split()[0]
    home = tempfile.mkdtemp(prefix="gitai-home-")
    env = dict(os.environ, HOME=home, GIT_AI_ALLOW_SUPERUSER="1", GIT_CONFIG_NOSYSTEM="1")
    env.pop("GIT_CONFIG_GLOBAL", None)
    for meta_path in sorted(out.glob("v1_*.meta.json")):
        name = meta_path.name[: -len(".meta.json")]
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory(prefix="v1-") as repo:
            run(["git", "init", "-q", repo])
            run(["git", "-C", repo, "fetch", "-q", "--update-head-ok", str(out / f"{name}.bundle"), "+refs/*:refs/*"])
            run(["git", "-C", repo, "checkout", "-q", "--detach", meta["head"]])
            files = {}
            for path in meta["paths"]:
                result = subprocess.run(
                    [git_ai, "blame", "--json", path], cwd=repo, env=env, capture_output=True
                )
                if result.returncode != 0:
                    sys.exit(f"{name} {path}: git-ai blame 실패\n{result.stderr.decode()}")
                value = json.loads(result.stdout.decode("utf-8"))
                prompts = {
                    key: {"agent_id": record["agent_id"], "human_author": record.get("human_author")}
                    for key, record in value["prompts"].items()
                }
                files[path] = {"lines": value["lines"], "prompts": prompts}
        expected = {
            "git_ai_version": version,
            "notes_git_ai_version": meta["git_ai_version"],
            "head": meta["head"],
            "files": files,
        }
        (out / f"{name}.json").write_text(
            json.dumps(expected, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        meta_path.unlink()
        ai = sum(1 for f in files.values() for _ in f["lines"])
        print(f"{name}: 파일 {len(files)}개, AI 범위 {ai}개")


if __name__ == "__main__":
    main()
