#!/usr/bin/env node
/**
 * 构建脚本:生成静态HTML文件用于生产环境
 * 这个脚本会:
 * 1. 使用EJS编译模板为静态HTML
 * 2. 替换脚本引用为打包后的bundle
 * 3. 将HTML输出到dist目录
 */

const ejs = require('ejs');
const fs = require('fs');
const path = require('path');

const distDir = path.join(__dirname, 'dist');
const viewsDir = path.join(__dirname, 'views');

// 确保dist目录存在
if (!fs.existsSync(distDir)) {
    fs.mkdirSync(distDir, { recursive: true });
}

console.log('开始构建渲染进程HTML...');

// 将 5 个 CSS 合并成单文件，省 4 次 app:// 协议往返
const cssDir = path.join(__dirname, 'assets', 'css');
const cssOrder = ['main.css', 'connection-dialog.css', 'file-manager.css', 'terminal.css', 'buttons.css'];
const bundledCss = cssOrder.map(f => `/* ${f} */\n${fs.readFileSync(path.join(cssDir, f), 'utf8')}`).join('\n\n');
const bundleOutDir = path.join(distDir, 'assets', 'css');
fs.mkdirSync(bundleOutDir, {recursive: true});
fs.writeFileSync(path.join(bundleOutDir, 'bundle.css'), bundledCss);
console.log(`✓ CSS bundle: ${(bundledCss.length / 1024).toFixed(1)} KB`);

// 编译EJS模板
ejs.renderFile(
    path.join(viewsDir, 'index.ejs'),
    {
        title: 'SSHL客户端',
        connections: [],
        basePath: __dirname,
        rendererScript: 'app://dist/assets/js/renderer.js',
        cssBundle: 'app://dist/assets/css/bundle.css' // 生产用合并 CSS
    },
    { root: viewsDir },
    (err, html) => {
        if (err) {
            console.error('EJS编译错误:', err);
            process.exit(1);
        }

        // 将HTML写入dist目录
        const outputPath = path.join(distDir, 'index.html');
        fs.writeFileSync(outputPath, html);

        console.log(`✓ HTML已生成: ${outputPath}`);
        console.log('✓ 脚本引用: app://dist/assets/js/renderer.js');
    }
);
