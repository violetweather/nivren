# Nivren OCI image

The supported image runs `niv` as non-root user/group 10001 in `/workspace` and includes the operating-system certificate roots needed by Nivren's verified TLS clients. It contains no compiler build tree or package-manager cache.

```sh
docker build -f containers/Dockerfile -t nivren:local .
docker run --rm nivren:local version
docker run --rm -v "$PWD:/workspace" nivren:local check .
```

Mount source read-only unless the command must write `target`, documentation, lockfiles, or snapshots. Project capability grants and resource limits remain enforced inside the container. Container isolation is additional defense, not a replacement for Nivren policy.
