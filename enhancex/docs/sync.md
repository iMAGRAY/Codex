# Upstream Synchronization

1. `git checkout Enhancex`
2. `git fetch upstream`
3. `git merge upstream/main`
4. `./enhancex/scripts/apply_overlays.sh`
5. `cargo test -p codex-apply-patch`
6. `./enhancex/scripts/test_begin_patch.sh` (to be implemented)
7. `git status` should show no conflicts; commit the updated overlays if necessary.
