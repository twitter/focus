.DEFAULT_GOAL = all

APACHE_TWGIT_BACKEND := git-server/apache2/cgi-bin/twgit-backend
TWGIT_BACKEND_BIN := twgit-utils/dist/twgit-backend

.PHONY: $(TWGIT_BACKEND_BIN)
$(TWGIT_BACKEND_BIN):
	make -C twgit-utils all

$(APACHE_TWGIT_BACKEND): $(TWGIT_BACKEND_BIN)
	cp -f $(TWGIT_BACKEND_BIN) $(APACHE_TWGIT_BACKEND)


all: $(APACHE_TWGIT_BACKEND)
