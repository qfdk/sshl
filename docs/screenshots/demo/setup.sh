#!/bin/bash
# 演示站点 /var/www/acme：文件、日期、git 历史、访问日志
set -e
A=/var/www/acme; mkdir -p $A/{config,logs,public,releases,scripts}; cd $A
git init -q -b main; git config user.name deploy; git config user.email deploy@acme.example
c(){ GIT_AUTHOR_DATE="$1" GIT_COMMITTER_DATE="$1" git commit -q --allow-empty-message -m "$2"; }
printf '# acme\n\nMarketing site and API gateway for acme.example.\n' > README.md; git add README.md; c "2026-09-02T09:14:00" "Initial commit"
printf '#!/bin/sh\nset -e\nrelease=$(date +%%Y%%m%%d%%H%%M)\nrsync -a build/ releases/$release/\nln -sfn releases/$release public/current\nsudo systemctl reload nginx\necho "deployed $release"\n' > deploy.sh; chmod 775 deploy.sh; git add deploy.sh; c "2026-09-18T16:40:00" "Add deploy script"
printf 'upstream api { server 127.0.0.1:8080; }\nlocation /healthz { return 200; }\n' > config/nginx.conf; git add config; c "2026-10-07T11:02:00" "Add health check for the API gateway"
printf 'gzip on;\ngzip_types text/css application/json;\n' >> config/nginx.conf; git add config; c "2026-10-21T14:25:00" "nginx: enable gzip"
echo 1.4.2 > VERSION; git add VERSION; c "2026-11-09T10:30:00" "Release 1.4.2"; git tag v1.4.2
printf 'logs/\nreleases/\n' > .gitignore; git add .gitignore; c "2026-11-09T10:31:00" "Ignore logs and release archives"
echo '<h1>acme</h1>' > public/index.html; echo 'echo backup' > scripts/backup.sh; mkdir -p releases/202611091030
cat > logs/access.log <<'LOG'
198.51.100.7 - - [17/Nov/2026:08:41:09 +0000] "GET /api/v1/status HTTP/2.0" 200 87
203.0.113.24 - - [17/Nov/2026:08:41:12 +0000] "GET /assets/app.css HTTP/2.0" 304 0
192.0.2.146 - - [17/Nov/2026:08:41:15 +0000] "POST /api/v1/login HTTP/2.0" 401 43
198.51.100.7 - - [17/Nov/2026:08:41:21 +0000] "GET /pricing HTTP/2.0" 200 8830
LOG
chmod 775 config logs public releases scripts; chmod 664 README.md VERSION .gitignore
touch -d "2026-09-02 09:14" README.md public; touch -d "2026-09-18 16:40" deploy.sh; touch -d "2026-10-21 14:25" config
touch -d "2026-10-07 11:02" scripts; touch -d "2026-11-09 10:30" releases VERSION .gitignore; touch -d "2026-11-17 08:41" logs; touch -d "2026-11-09 10:31" .git
chown -R deploy:deploy /var/www/acme; touch -d "2026-11-09 10:33" $A
