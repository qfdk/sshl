# sudo 上传 / 文件管理跟随终端用户 集成测试用的服务器：alice（sudo 需密码）、bob（目标目录属主，密码 bobpw）、
# carol（无 sudo）、dave（免密 sudo）、root（密码 rootpw，供 su）。
# 用法见 src-tauri/src/sftp.rs 的 sudo_upload_against_real_linux。
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server sudo && rm -rf /var/lib/apt/lists/* \
 && echo 'root:rootpw' | chpasswd \
 && useradd -m -s /bin/bash alice && echo 'alice:alicepw' | chpasswd && usermod -aG sudo alice \
 && useradd -m -s /bin/bash bob && echo 'bob:bobpw' | chpasswd && mkdir -p /home/bob/uploads/existing && echo keep > /home/bob/uploads/existing/keep.txt \
 && chown -R bob:bob /home/bob/uploads && chmod 755 /home/bob /home/bob/uploads \
 && useradd -m -s /bin/bash carol && echo 'carol:carolpw' | chpasswd \
 && useradd -m -s /bin/bash dave && echo 'dave:davepw' | chpasswd && echo 'dave ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/dave \
 && mkdir -p /run/sshd && printf 'PasswordAuthentication yes\nKbdInteractiveAuthentication no\n' > /etc/ssh/sshd_config.d/test.conf
EXPOSE 22
CMD ["/usr/sbin/sshd","-D","-e"]
