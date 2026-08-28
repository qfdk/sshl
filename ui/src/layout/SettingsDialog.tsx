export function SettingsDialog() {
  return (
    <div id="settings-dialog" className="dialog">
      <div className="dialog-content settings-dialog-content">
        <div className="settings-header">
          <h3>设置</h3>
          <button type="button" id="settings-close" className="icon-button" title="关闭">
            <i data-lucide="x" data-size="16" />
          </button>
        </div>

        <div className="settings-layout">
          <nav className="settings-sidebar" role="tablist" aria-label="设置分类">
            <button type="button" className="settings-tab active" data-settings-tab="font" role="tab" aria-selected="true">
              <i data-lucide="type" data-size="15" /><span>字体</span>
            </button>
            <button type="button" className="settings-tab" data-settings-tab="groups" role="tab" aria-selected="false">
              <i data-lucide="folder" data-size="15" /><span>分组管理</span>
            </button>
          </nav>
          <div className="settings-panels">
            <section id="settings-panel-font" className="settings-panel" data-settings-panel="font">
              <div className="settings-panel-heading">
                <h4>字体</h4>
                <p>调整终端字号和字体，预览会实时更新。</p>
              </div>
              <form id="settings-form" className="settings-panel-form">
                <div className="settings-panel-body">
                  <div className="settings-row">
                    <div className="settings-field">
                      <label htmlFor="settings-font-size">字号</label>
                      <div className="font-size-control">
                        <button type="button" className="font-size-step" data-step="-1" aria-label="减小">−</button>
                        <input type="number" id="settings-font-size" min="8" max="40" step="1" required autoComplete="off" />
                        <button type="button" className="font-size-step" data-step="1" aria-label="增大">+</button>
                      </div>
                    </div>
                    <div className="settings-field settings-field-grow">
                      <label htmlFor="settings-font-family">字体</label>
                      <select id="settings-font-family" />
                    </div>
                  </div>

                  <div className="settings-field settings-custom-row hidden" id="settings-custom-wrap">
                    <label htmlFor="settings-font-family-custom">自定义 fontFamily</label>
                    <input type="text" id="settings-font-family-custom" placeholder='"Cascadia Code", monospace' autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                  </div>

                  <div className="settings-preview" id="settings-preview">
                    <div className="settings-preview-line">user@host:~$ ls -la /etc</div>
                    <div className="settings-preview-line dim">drwxr-xr-x  123 root  4096 May 24 10:42 .</div>
                    <div className="settings-preview-line">The quick brown fox 0123456789 → ✓</div>
                    <div className="settings-preview-line">Emoji: 😀 😃 🚀 🐱 ❤️ 🌈 🍎 ✅ ⚡ 中文</div>
                    <div className="settings-preview-line">Nerd Font 图标: &#xe0b0; &#xf07b; &#xf015; &#xf120; &#xe0a0; &#xf09b; &#xf135;</div>
                  </div>
                </div>
                <div className="settings-panel-footer">
                  <button type="button" id="settings-cancel">取消</button>
                  <button type="submit" id="settings-save"><i data-lucide="check" data-size="16" />保存</button>
                </div>
              </form>
            </section>

            <section id="settings-panel-groups" className="settings-panel" data-settings-panel="groups" hidden>
              <div className="settings-panel-heading">
                <h4>分组管理</h4>
                <p>整理机器分组，拖拽连接列表即可调整顺序。</p>
              </div>
              <form id="group-manager-form" className="settings-panel-form">
                <div className="settings-panel-body">
                  <div className="group-manager-create">
                    <input type="text" id="group-new-name" placeholder="新分组名称" autoComplete="off" maxLength={40} />
                    <button type="button" id="group-add-btn"><i data-lucide="plus" data-size="15" />添加分组</button>
                  </div>
                  <div className="group-manager-section">
                    <div className="group-manager-section-title">已有分组</div>
                    <div id="group-manager-groups" />
                  </div>
                  <div className="group-manager-section">
                    <div className="group-manager-section-title">快速分配机器</div>
                    <div id="group-manager-connections" />
                  </div>
                </div>
                <div className="settings-panel-footer">
                  <button type="submit" id="group-manager-save"><i data-lucide="check" data-size="16" />保存</button>
                </div>
              </form>
            </section>
          </div>
        </div>
      </div>
    </div>
  );
}
