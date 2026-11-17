#!/bin/bash
service nginx start >/dev/null 2>&1; service redis-server start >/dev/null 2>&1; service postgresql start >/dev/null 2>&1; service cron start >/dev/null 2>&1
exec /usr/sbin/sshd -D -e
