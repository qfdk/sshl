# README 截图用的演示服务器：deploy 用户、/var/www/acme 站点（带 git 历史和访问日志）、nginx / redis / postgres 在跑。
# 用法（在本目录）：
#   cp <演示用的公钥> authorized_keys          # 演示环境的 HOME 是 /Users/Shared/sshl-demo，公钥在它的 .ssh/id_ed25519.pub
#   docker build -t sshl-demo .
#   docker run -d --rm --name sshl-demo-web-01 --hostname web-01 -p 127.0.0.1:2201:22 sshl-demo
#   （web-02 / db-01 / cache-01 / stg-01 / stg-db 依次用 2202 … 2206；连接里的主机写 web-01.localhost 这样的名字）
# 系统页要有点负载才好看：docker exec -d sshl-demo-web-01 redis-benchmark -q -l -c 3 -t set,get -r 3000000 -d 120
# 截图：HOME=/Users/Shared/sshl-demo SSHL_DEV_SOCKET=… SSHL_DEV_SIZE=1440x880 起带 devhook 的 release 版，
# `language zh|en`、`appearance dark|light` 切语言和外观，screencapture -x -o -l <窗口号>（要在 2 倍像素的屏上、窗口没被盖住）。
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server git nginx redis-server postgresql cron iproute2 procps less ca-certificates rsync \
 && rm -rf /var/lib/apt/lists/* && mkdir -p /run/sshd \
 && useradd -m -s /bin/bash deploy && mkdir -p /home/deploy/.ssh \
 && printf 'PasswordAuthentication no\nPrintMotd no\nPrintLastLog no\n' > /etc/ssh/sshd_config.d/demo.conf \
 && rm -f /etc/update-motd.d/* /etc/legal && touch /home/deploy/.hushlogin \
 && sed -i 's/^#force_color_prompt=yes/force_color_prompt=yes/' /home/deploy/.bashrc && echo 'cd /var/www/acme' >> /home/deploy/.bashrc
COPY authorized_keys /home/deploy/.ssh/authorized_keys
COPY setup.sh start.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/*.sh && chown -R deploy:deploy /home/deploy && chmod 700 /home/deploy/.ssh && chmod 600 /home/deploy/.ssh/authorized_keys && /usr/local/bin/setup.sh
EXPOSE 22
CMD ["/usr/local/bin/start.sh"]
