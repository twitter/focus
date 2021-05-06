bazel query 'buildfiles(deps(language-server/log-collector/...))' --output package | grep -v '^@' | grep -v '^external$'

