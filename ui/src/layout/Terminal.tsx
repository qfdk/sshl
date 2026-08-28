export function Terminal() {
  return (
    <div className="terminal-view">
      <div className="terminal-content">
        <div id="terminal-container" className="terminal-container" />
        <div id="terminal-placeholder" className="terminal-placeholder">
          <div className="placeholder-content">
            <p>点击左侧连接或创建新连接开始使用</p>
          </div>
        </div>
      </div>
    </div>
  );
}
