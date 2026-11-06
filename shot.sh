#!/bin/sh
# 截 SSHL 窗口（点坐标 6,33 1908x914），缩到 1x 宽度
D=$(dirname "$0"); OUT=$D/$1.png
screencapture -x -R6,33,1908,914 $OUT && sips -Z 1600 $OUT >/dev/null && echo $OUT
