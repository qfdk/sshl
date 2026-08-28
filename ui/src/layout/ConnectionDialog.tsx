export function ConnectionDialog() {
  return (
    <>
      <div id="connection-dialog" className="dialog">
      <div className="dialog-content">
        <h3>新建SSH连接</h3>
        <form id="connection-form">
          <div className="form-group">
            <label htmlFor="conn-name">名称</label>
            <input type="text" id="conn-name" placeholder="我的服务器" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
          </div>
          <div className="form-group">
            <label htmlFor="conn-group">分组 <span className="form-hint">（可选）</span></label>
            <input type="text" id="conn-group" placeholder="例如：生产环境、开发环境" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
          </div>
          <div className="form-row">
            <div className="form-group form-group-grow">
              <label htmlFor="conn-host">主机</label>
              <input type="text" id="conn-host" placeholder="example.com 或 192.168.1.1" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="form-group form-group-port">
              <label htmlFor="conn-port">端口</label>
              <input type="number" id="conn-port" defaultValue="22" min="1" max="65535" autoComplete="off" />
            </div>
          </div>
          <div className="form-group">
            <label htmlFor="conn-username">用户名</label>
            <input type="text" id="conn-username" placeholder="root" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
          </div>

          <div className="form-group">
            <label htmlFor="auth-type">认证方式</label>
            <select id="auth-type">
              <option value="password">密码</option>
              <option value="privateKey">私钥</option>
            </select>
          </div>

          <div className="form-group auth-password">
            <label htmlFor="conn-password">密码</label>
            <input type="password" id="conn-password" autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
          </div>

          <div className="form-group auth-key hidden">
            <label htmlFor="conn-private-key">私钥文件</label>
            <div className="file-input-group">
              <input type="text" id="conn-private-key-path" readOnly autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              <button type="button" id="browse-private-key"><i data-lucide="folder-open" data-size="14" />浏览</button>
            </div>
          </div>

          <div className="form-group auth-key hidden">
            <label htmlFor="conn-passphrase">私钥密码 (可选)</label>
            <input type="password" id="conn-passphrase" autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
          </div>

          <div className="form-group">
            <div className="checkbox-container">
              <input type="checkbox" id="conn-save-password" defaultChecked />
              <label htmlFor="conn-save-password">保存密码</label>
            </div>
          </div>

          <div className="dialog-buttons">
            <button type="button" id="cancel-connection"><i data-lucide="x" data-size="16" />取消</button>
            <button type="submit" id="connection-submit-btn"><i data-lucide="arrow-right" data-size="16" />连接</button>
          </div>
        </form>
      </div>
    </div>

      <div id="loading-overlay" className="loading-overlay hidden">
        <div className="spinner" />
        <div className="loading-text">连接中...</div>
      </div>
    </>
  );
}
