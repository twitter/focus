FROM httpd:2.4

COPY ./90_git /etc/apt/preferences.d/90_git
COPY ./backports.list /etc/apt/sources.list.d/backports.list

RUN apt update \
    && apt upgrade -y \
    && apt install --no-install-recommends -y git python3 \
    && groupadd -g 503 slyphon \
    && useradd -u 503 -g 503 -d /nonexistent --no-create-home --no-user-group \
        --shell /usr/sbin/nologin --password '!' slyphon \
    && rm -rf /var/lib/apt/lists/*
