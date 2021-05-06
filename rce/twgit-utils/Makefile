
.DEFAULT_GOAL = all
.PHONY: clean

DIST_DIR := dist
TWGIT_CGI_BIN := ../git-server/apache2/cgi-bin/twgit-backend
TWGIT_MAIN := $(DIST_DIR)/twgit
GIT_REMOTE_TWGIT := $(DIST_DIR)/git-remote-twgit
TWGIT_BACKEND := $(DIST_DIR)/twgit-backend
COVERAGE_DIR := .coverage

ALL_GO_FILES := $(shell find . -name '*.go' -type f)

$(DIST_DIR):
	mkdir -p $(DIST_DIR)

$(TWGIT_MAIN): $(DIST_DIR) $(ALL_GO_FILES)
	go build -o $(TWGIT_MAIN) .

$(GIT_REMOTE_TWGIT): $(TWGIT_MAIN)
	ln -f $(TWGIT_MAIN) $(GIT_REMOTE_TWGIT)

twgit-main: $(TWGIT_MAIN)

$(TWGIT_CGI_BIN): $(TWGIT_BACKEND)
	ln -f $(TWGIT_BACKEND) $(TWGIT_CGI_BIN)

$(TWGIT_BACKEND): backend/main.go internal/backend/backend.go
	go build -o $(TWGIT_BACKEND) ./backend

$(COVERAGE_DIR):
	mkdir -p $(COVERAGE_DIR)

.PHONY: test
test: $(COVERAGE_DIR)
	go test -coverprofile=cover.out -covermode=set ./...
	go tool cover -html=cover.out -o $(COVERAGE_DIR)/index.html

.PHONY: coversvc
coversvc:
	go run . cover .coverage

clean:
	@rm -rf dist/*

all: $(TWGIT_MAIN) $(GIT_REMOTE_TWGIT) $(TWGIT_BACKEND) $(TWGIT_CGI_BIN)
