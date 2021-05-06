package testutils

import (
	"os"
	"testing"

	r "github.com/stretchr/testify/require"
)

func TestLoadRelativeFile(t *testing.T) {
	fix := NewFixture(t)
	data := fix.ReadProjectRootRelativeFile("internal", "testutils", "bogus.go")

	r.Contains(t, string(data), "// MARK\n")
}

func TestRootRelativeJoin(t *testing.T) {
	f := NewFixture(t)
	// kind of a lame test, but in theory testing that we exist probably is OK
	path := f.RootRelativeJoin("internal", "testutils", "fixture_test.go")
	_, err := os.Stat(path)
	f.NoError(err)
}
