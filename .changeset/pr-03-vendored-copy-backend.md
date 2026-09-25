---
"skills-manager": minor
---

pr: #3

Add a vendored copy mode for projects. Skill files are copied into `<project>/.agents/skills` and committed with the repo, and every other agent gets a relative symlink to that copy, so a fresh clone works on any machine without the library. Existing link-mode projects can be converted, with a preview first. On Windows, the agent links need Developer Mode or the symlink privilege.
