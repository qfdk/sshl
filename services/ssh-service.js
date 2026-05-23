const {Client} = require('ssh2');
const {EventEmitter} = require('events');
const fs = require('fs');
const path = require('path');
const os = require('os');
const net = require('net');
const crypto = require('crypto');

// 把任意字符串安全地包成 POSIX shell 单引号字面量，防注入
function shellQuote(str) {
    return `'${String(str).replace(/'/g, `'\\''`)}'`;
}

// 严格校验：八进制权限串（3-4 位 0-7）
function isValidOctalPermissions(p) {
    return typeof p === 'string' && /^[0-7]{3,4}$/.test(p);
}

// 严格校验：Unix 用户名/组名
function isValidUnixName(n) {
    return typeof n === 'string' && /^[A-Za-z_][A-Za-z0-9_.-]{0,31}$/.test(n);
}

// 已知主机指纹文件（TOFU 信任模型）
const KNOWN_HOSTS_FILE = path.join(os.homedir(), '.sshl', 'known_hosts.json');
function loadKnownHosts() {
    try {
        return JSON.parse(fs.readFileSync(KNOWN_HOSTS_FILE, 'utf8')) || {};
    } catch {
        return {};
    }
}
function saveKnownHost(hostKey, fingerprint) {
    try {
        const data = loadKnownHosts();
        data[hostKey] = fingerprint;
        const dir = path.dirname(KNOWN_HOSTS_FILE);
        if (!fs.existsSync(dir)) fs.mkdirSync(dir, {recursive: true, mode: 0o700});
        fs.writeFileSync(KNOWN_HOSTS_FILE, JSON.stringify(data, null, 2), {mode: 0o600});
    } catch (e) {
        console.warn('保存 known_hosts 失败:', e.message);
    }
}

/**
 * SSH服务类 - 使用单例模式管理SSH连接
 */
class SshService extends EventEmitter {
    constructor() {
        super();
        this.sessions = new Map(); // 存储会话信息
        this.connectionPool = new Map(); // 存储连接池
        this.connectionToSession = new Map(); // 存储连接ID到会话ID的映射
        this.sftpStatus = new Map(); // 存储连接的SFTP可用性状态
        this.execStatus = new Map(); // 存储连接的exec可用性状态
        this.commandQueue = new Map(); // 存储每个连接的命令队列
        this.fileManagerSessions = new Map(); // 存储连接到文件管理会话的映射
        this.userInfoCache = new Map(); // 存储UID/GID到用户名/组名的映射缓存

        // 设置定期清理过期连接
        this._cleanupTimer = setInterval(() => this.cleanExpiredConnections(), 30000); // 每30秒清理一次
    }

    /**
     * 预热连接：提前建立 TCP+SSH 通道放入连接池，用户点击时直接复用，
     * 跳过 ~700ms 的握手 + sshd fork 冷启动。
     * 不创建 session，不占 refCount，10 分钟无人使用按正常逻辑回收。
     * @param {Object} connectionDetails
     * @returns {Promise<boolean>}
     */
    async prewarmConnection(connectionDetails) {
        try {
            if (!connectionDetails?.host || !connectionDetails.username) return false;
            const t0 = Date.now();
            const result = await this.getOrCreateConnection(connectionDetails);
            if (!result.isNew) {
                console.log(`[prewarm] ${connectionDetails.host} 已在池中`);
                return true;
            }
            await this.setupConnection(result.conn, result.connectionObj, result.connectOptions, result.connectionKey);
            // setupConnection 在 ready 时 refCount++ 设为 1；预热不占用，归零让正常清理逻辑生效
            result.connectionObj.refCount = 0;
            console.log(`[timing] prewarm ${connectionDetails.host} 完成 ${Date.now() - t0}ms`);
            return true;
        } catch (e) {
            console.warn(`[prewarm] ${connectionDetails?.host} 失败:`, e.message);
            return false;
        }
    }

    /**
     * 销毁服务：清理定时器并断开所有连接
     */
    dispose() {
        if (this._cleanupTimer) {
            clearInterval(this._cleanupTimer);
            this._cleanupTimer = null;
        }
        for (const [, conn] of this.connectionPool.entries()) {
            try { conn.client?.end(); } catch {}
        }
        this.connectionPool.clear();
        this.sessions.clear();
    }

    /**
     * 清理过期的连接
     * @private
     */
    cleanExpiredConnections() {
        const now = Date.now();
        const expireTime = 10 * 60 * 1000; // 10分钟过期时间

        for (const [key, conn] of this.connectionPool.entries()) {
            if (now - conn.lastUsed > expireTime && conn.refCount <= 0) {
                console.log(`清理过期连接: ${key}`);
                if (conn.client && conn.client.end) {
                    conn.client.end();
                }
                this.connectionPool.delete(key);
                this.sftpStatus.delete(key); // 同时清理SFTP状态
                this.userInfoCache.delete(key); // 清理用户信息缓存
                this.execStatus.delete(key); // 同时清理exec状态
                this.fileManagerSessions.delete(key); // 同时清理文件管理会话映射
            }
        }
    }

    /**
     * 根据连接ID获取会话信息
     * @param {string} connectionId - 连接ID
     * @returns {Object|null} - 会话信息
     */
    getSessionByConnectionId(connectionId) {
        if (!connectionId) return null;

        const sessionId = this.connectionToSession.get(connectionId);
        if (!sessionId) return null;

        // 检查会话是否仍然存在
        const session = this.sessions.get(sessionId);
        if (!session) {
            // 如果会话不存在，清理映射
            this.connectionToSession.delete(connectionId);
            return null;
        }

        return {
            sessionId,
            session
        };
    }

    /**
     * 根据连接详情获取或创建连接
     * @param {Object} connectionDetails - 连接详情
     * @returns {Promise<Object>} - 连接对象和相关信息
     */
    async getOrCreateConnection(connectionDetails) {
        const connectionKey = `${connectionDetails.username}@${connectionDetails.host}:${connectionDetails.port || 22}`;
        
        // 检查连接池中是否已存在此连接
        if (this.connectionPool.has(connectionKey)) {
            const connectionObj = this.connectionPool.get(connectionKey);
            // 检查连接是否仍然活跃
            if (connectionObj.isConnected) {
                console.log(`复用现有连接: ${connectionKey}`);
                connectionObj.lastUsed = Date.now(); // 更新最后使用时间
                return { conn: connectionObj.client, connectionObj, isNew: false, connectionKey };
            }
        }
        
        // 创建新连接
        const client = new Client();
        const conn = {
            client,
            isConnected: false,
            refCount: 0,
            lastUsed: Date.now()
        };
        
        // 创建连接配置
        const connectOptions = this.createConnectOptions(connectionDetails);
        
        // 返回连接前不等待连接成功，将在后续connect中等待
        return { conn: client, connectionObj: conn, isNew: true, connectionKey, connectOptions };
    }

    /**
     * 创建连接配置
     * @param {Object} connectionDetails - 连接详情
     * @returns {Object} - 连接配置
     * @private
     */
    createConnectOptions(connectionDetails) {
        const hostKey = `${connectionDetails.host}:${connectionDetails.port || 22}`;
        // TOFU host key 校验：首次连接保存指纹，后续比对，变化则拒绝
        const hostVerifier = (key) => {
            const fp = crypto.createHash('sha256').update(key).digest('base64');
            const known = loadKnownHosts();
            const expected = known[hostKey];
            if (!expected) {
                console.log(`首次连接 ${hostKey}，保存主机指纹 SHA256:${fp}`);
                saveKnownHost(hostKey, fp);
                return true;
            }
            if (expected !== fp) {
                console.error(`主机指纹不匹配 ${hostKey}: 期望 ${expected}, 实际 ${fp}`);
                this.emit('host-key-mismatch', {hostKey, expected, actual: fp});
                return false;
            }
            return true;
        };

        // 连接配置
        const connectOptions = {
            host: connectionDetails.host,
            port: connectionDetails.port || 22,
            username: connectionDetails.username,
            // 设置终端类型，以确保正确的shell环境
            term: 'xterm-color',
            // 连接超时设置
            readyTimeout: 30000,
            // SSH2官方推荐的keepalive设置
            keepaliveInterval: 30000, // 30秒，避免过于频繁
            keepaliveCountMax: 3, // 最多3次keepalive失败后断开
            // 添加错误处理级别配置
            debug: false, // 生产环境关闭调试
            hostVerifier
            // 算法：使用 ssh2 默认值（包含 curve25519/aes-gcm 等现代快算法，且与 OpenSSH 默认兼容）
        };

        // 局域网仅放宽超时，不强制绑定 localAddress（避免选错接口导致首次连接慢/超时）
        if (/^(192\.168\.|10\.|172\.16\.)/.test(connectionDetails.host)) {
            connectOptions.readyTimeout = 60000;
        }

        // 根据认证类型选择认证方式
        if (connectionDetails.authType === 'privateKey' && connectionDetails.privateKey) {
            try {
                connectOptions.privateKey = fs.readFileSync(connectionDetails.privateKey);
                if (connectionDetails.passphrase) {
                    connectOptions.passphrase = connectionDetails.passphrase;
                }
            } catch (err) {
                throw new Error(`读取密钥文件失败: ${err.message}`);
            }
        } else if (connectionDetails.password) {
            connectOptions.password = connectionDetails.password;
        } else {
            throw new Error('需要提供密码或私钥');
        }

        return connectOptions;
    }

    /**
     * 建立连接并初始化事件监听
     * @param {Object} conn - SSH连接客户端
     * @param {Object} connectionObj - 连接对象
     * @param {Object} connectOptions - 连接配置
     * @param {string} connectionKey - 连接唯一标识
     * @returns {Promise<void>}
     * @private
     */
    setupConnection(conn, connectionObj, connectOptions, connectionKey) {
        const t0 = Date.now();
        console.log(`[timing] ${connectionKey} 开始 TCP+SSH 握手`);

        // 关键性能优化：自己建 socket 并禁用 Nagle。
        // ssh2 默认不设 TCP_NODELAY，Nagle 会让 channel/pty/shell 等多 RTT 协议每步多 ~40ms 延迟，
        // 实测 fro2 服务器从 414ms → 150ms（2.7x 提速）。OpenSSH 默认就是 NODELAY。
        if (!connectOptions.sock) {
            const sock = net.connect({host: connectOptions.host, port: connectOptions.port});
            sock.setNoDelay(true);
            sock.on('error', () => { /* 让 ssh2 通过 client error 报告即可 */ });
            connectOptions.sock = sock;
        }

        return new Promise((resolve, reject) => {
            // 设置事件监听
            conn.on('ready', () => {
                // 连接成功，加入连接池
                connectionObj.isConnected = true;
                connectionObj.refCount++;
                this.connectionPool.set(connectionKey, connectionObj);
                console.log(`[timing] ${connectionKey} ready 耗时 ${Date.now() - t0}ms, refCount=${connectionObj.refCount}`);
                resolve();
            });
            
            conn.on('error', (err) => {
                // 连接失败处理，根据错误级别处理
                const errorLevel = err.level || 'unknown';
                console.error(`连接错误 [${errorLevel}]: ${connectionKey}`, err.message);
                
                // 根据错误级别决定处理策略
                if (errorLevel === 'client-socket' || errorLevel === 'client-ssh') {
                    connectionObj.isConnected = false;
                    // 移除失败的连接
                    if (this.connectionPool.get(connectionKey) === connectionObj) {
                        this.connectionPool.delete(connectionKey);
                        this.sftpStatus.delete(connectionKey);
                        this.execStatus.delete(connectionKey);
                    }
                }
                reject(err);
            });
            
            conn.on('end', () => {
                // 连接结束处理
                console.log(`连接结束: ${connectionKey}`);
                connectionObj.isConnected = false;
                connectionObj.refCount = 0;
                // 移除已结束的连接
                if (this.connectionPool.get(connectionKey) === connectionObj) {
                    this.connectionPool.delete(connectionKey);
                }
            });
            
            conn.on('close', () => {
                // 连接关闭处理
                console.log(`连接关闭: ${connectionKey}`);
                connectionObj.isConnected = false;
                connectionObj.refCount = 0;
                // 移除已关闭的连接
                if (this.connectionPool.get(connectionKey) === connectionObj) {
                    this.connectionPool.delete(connectionKey);
                }
            });
            
            // 连接
            conn.connect(connectOptions);
        });
    }


    /**
     * 等待会话 buffer 出现首块数据（或超时）。
     * 用于在 connect() 返回前确保 welcome/PS1 已落入 buffer，避免 UI 黑屏间隙。
     * @param {string} sessionId
     * @param {number} timeoutMs
     */
    waitForFirstData(sessionId, timeoutMs = 400) {
        return new Promise((resolve) => {
            const session = this.sessions.get(sessionId);
            if (!session || (session.buffer && session.buffer.length > 0)) {
                return resolve();
            }
            const stream = session.stream;
            if (!stream) return resolve();

            let done = false;
            const finish = () => {
                if (done) return;
                done = true;
                stream.removeListener('data', onData);
                stream.stderr?.removeListener?.('data', onData);
                clearTimeout(timer);
                resolve();
            };
            const onData = () => finish();
            const timer = setTimeout(finish, timeoutMs);
            stream.once('data', onData);
            stream.stderr?.once?.('data', onData);
        });
    }

    /**
     * 创建shell会话
     * @param {string} sessionId - 会话ID
     * @param {Object} conn - SSH连接客户端
     * @returns {Promise<Object>} - shell流
     * @private
     */
    createShellSession(sessionId, conn) {
        return new Promise((resolve, reject) => {
            console.log(`创建shell会话: ${sessionId}`);
            conn.shell({term: 'xterm-color', rows: 24, cols: 80}, (err, stream) => {
                if (err) {
                    console.error(`创建shell失败: ${sessionId}`, err);
                    reject(err);
                    return;
                }

                // 设置数据处理
                this.setupStreamHandlers(sessionId, stream);
                resolve(stream);
            });
        });
    }

    /**
     * 设置流事件处理
     * @param {string} sessionId - 会话ID
     * @param {Object} stream - shell流
     * @private
     */
    setupStreamHandlers(sessionId, stream) {
        // 创建数据批处理器
        const createBatchProcessor = () => {
            let pendingData = '';
            let batchTimer = null;
            const BATCH_DELAY = 16; // 约60fps的更新频率
            const MAX_BATCH_SIZE = 8192; // 8KB最大批处理大小
            
            return (data) => {
                pendingData += data;
                
                // 如果数据量大，立即发送
                if (pendingData.length >= MAX_BATCH_SIZE) {
                    if (batchTimer) {
                        clearTimeout(batchTimer);
                        batchTimer = null;
                    }
                    const dataToSend = pendingData;
                    pendingData = '';
                    return dataToSend;
                }
                
                // 否则批量处理
                if (!batchTimer) {
                    batchTimer = setTimeout(() => {
                        batchTimer = null;
                        if (pendingData) {
                            const dataToSend = pendingData;
                            pendingData = '';
                            const session = this.sessions.get(sessionId);
                            if (session && session.active) {
                                this.emit('data', sessionId, dataToSend);
                            }
                        }
                    }, BATCH_DELAY);
                }
                
                return null;
            };
        };
        
        const batchProcessor = createBatchProcessor();
        
        // 处理数据事件
        stream.on('data', (data) => {
            const dataStr = data.toString('utf8');
            const session = this.sessions.get(sessionId);
            if (session) {
                // 追加到缓冲区（限制大小）
                const MAX_BUFFER_SIZE = 102400; // 100KB
                session.buffer = (session.buffer || '') + dataStr;
                if (session.buffer.length > MAX_BUFFER_SIZE) {
                    session.buffer = session.buffer.slice(-MAX_BUFFER_SIZE);
                }
                this.sessions.set(sessionId, session);

                // 只有活跃会话才发送数据
                if (session.active) {
                    const batchedData = batchProcessor(dataStr);
                    if (batchedData) {
                        this.emit('data', sessionId, batchedData);
                    }
                }
            }
        });

        // 处理stderr数据
        stream.stderr.on('data', (data) => {
            const dataStr = data.toString('utf8');
            const session = this.sessions.get(sessionId);
            if (session) {
                session.buffer = (session.buffer || '') + dataStr;
                if (session.buffer.length > 102400) {
                    session.buffer = session.buffer.slice(-102400);
                }
                this.sessions.set(sessionId, session);

                if (session.active) {
                    this.emit('data', sessionId, dataStr);
                }
            }
        });

        // 处理流关闭事件
        stream.on('close', () => {
            console.log(`Stream关闭: ${sessionId}`);
            this.handleStreamClose(sessionId);
        });
    }

    /**
     * 处理流关闭事件
     * @param {string} sessionId - 会话ID
     * @private
     */
    handleStreamClose(sessionId) {
        this.emit('close', sessionId);

        const session = this.sessions.get(sessionId);
        if (!session) return;

        // 标记会话为非活跃
        session.active = false;
        session.stream = null;
        this.sessions.set(sessionId, session);
        
        // 更新连接池引用计数
        const connectionKey = session.connectionKey;
        const connectionObj = this.connectionPool.get(connectionKey);
        if (connectionObj) {
            connectionObj.refCount--;
            if (connectionObj.refCount <= 0) {
                console.log(`引用计数归零，准备清理连接: ${connectionKey}`);
                // 不立即清理，允许复用
                connectionObj.lastUsed = Date.now();
            }
        }

        console.log(`会话 ${sessionId} 已关闭，标记为非活跃`);
    }

    /**
     * 建立SSH连接
     * @param {Object} connectionDetails - 连接详情
     * @returns {Promise<Object>} - 会话信息
     */
    async connect(connectionDetails) {
        const tConnect = Date.now();
        try {
            if (!connectionDetails || !connectionDetails.host || !connectionDetails.username) {
                throw new Error('缺少必要的连接参数');
            }

            // 检查是否有可复用的会话
            if (connectionDetails.id) {
                const existingSessionInfo = this.getSessionByConnectionId(connectionDetails.id);
                if (existingSessionInfo && existingSessionInfo.session) {
                    const { sessionId, session } = existingSessionInfo;
                    console.log(`复用现有会话: ${sessionId}, 连接ID: ${connectionDetails.id}`);
                    
                    // 标记会话为活跃
                    session.active = true;
                    this.sessions.set(sessionId, session);
                    
                    // 更新连接最后使用时间
                    const connectionKey = session.connectionKey;
                    const connectionObj = this.connectionPool.get(connectionKey);
                    if (connectionObj) {
                        connectionObj.lastUsed = Date.now();
                    }
                    
                    // 如果会话有流但没有连接，尝试重新创建
                    if (!session.stream) {
                        try {
                            const result = await this.activateSession(sessionId);
                            return result.success ? { sessionId: result.sessionId } : { sessionId };
                        } catch (err) {
                            console.warn(`重新激活会话 ${sessionId} 失败，继续使用现有会话`);
                        }
                    }
                    
                    return { sessionId };
                }
            }

            // 创建会话ID
            const sessionId = crypto.randomUUID();

            // 获取或创建连接
            const { conn, connectionObj, isNew, connectionKey, connectOptions } = 
                await this.getOrCreateConnection(connectionDetails);
            
            // 创建会话对象
            // active 初始为 false：shell 启动早期数据只进 buffer 不 emit，
            // 避免和 renderer 端 getSessionBuffer 写入产生双写。
            // renderer 完成 initTerminal+写入 buffer 后调 activateSession 才开始 emit。
            // shellPending 表示 shell 通道仍在异步建立中，sendData/resize 此时入队。
            this.sessions.set(sessionId, {
                conn,
                stream: null,
                details: connectionDetails,
                connectionKey,
                connectionId: connectionDetails.id,
                active: false,
                buffer: '',
                bufferReadLen: 0,
                shellPending: true,
                pendingWrites: [],
                pendingResize: null
            });
            
            // 保存连接ID到会话ID的映射
            if (connectionDetails.id) {
                this.connectionToSession.set(connectionDetails.id, sessionId);
            }
            
            // 如果是新连接，需要连接
            if (isNew) {
                await this.setupConnection(conn, connectionObj, connectOptions, connectionKey);
                // 加入连接池
                this.connectionPool.set(connectionKey, connectionObj);
            } else {
                // 增加引用计数
                connectionObj.refCount++;
                // 更新最后使用时间
                connectionObj.lastUsed = Date.now();
            }
            
            // 等 shell 就绪再返回；异步 shell 会让 loading 关掉后出现黑屏空 xterm。
            const tShell = Date.now();
            const stream = await this.createShellSession(sessionId, conn);
            console.log(`[timing] shell 启动耗时 ${Date.now() - tShell}ms`);

            const session = this.sessions.get(sessionId);
            if (session) {
                session.stream = stream;
                session.shellPending = false;
                this.sessions.set(sessionId, session);
            }

            // shell 通道打开 != server 已发出 welcome/PS1。再等一小段，
            // 让首块数据落进 buffer，这样 renderer getSessionBuffer 就能拿到回显，
            // loading 关掉后立即可见，避免"空终端 → 闪烁光标 → 才出回显"。
            await this.waitForFirstData(sessionId, 400);

            console.log(`[timing] connect 总耗时 ${Date.now() - tConnect}ms (会话 ${sessionId})`);
            return { sessionId };
        } catch (error) {
            console.error('SSH连接失败:', error);
            throw error;
        }
    }

    /**
     * 断开SSH连接
     * @param {string} sessionId - 会话ID
     * @returns {Promise<boolean>} - 断开结果
     */
    async disconnect(sessionId) {
        const session = this.sessions.get(sessionId);
        if (!session) {
            throw new Error('会话未找到');
        }

        // 标记会话为非活跃
        session.active = false;
        this.sessions.set(sessionId, session);
        
        // 更新连接池引用计数
        const connectionKey = session.connectionKey;
        const connectionObj = this.connectionPool.get(connectionKey);
        
        if (connectionObj) {
            connectionObj.refCount--;
            connectionObj.lastUsed = Date.now();
            
            console.log(`断开会话 ${sessionId}, 连接 ${connectionKey} 引用计数: ${connectionObj.refCount}`);
            
            // 如果引用计数为0，但不立即关闭连接，允许后续复用
            if (connectionObj.refCount <= 0) {
                console.log(`连接 ${connectionKey} 引用计数为0，但保留连接以便复用`);
            }
        } else {
            // 如果连接对象已不存在，但会话仍然保留连接对象，关闭它
            if (session.conn && session.conn.end) {
                session.conn.end();
            }
        }
        
        return true;
    }

    /**
     * 确保会话处于活跃状态
     * @param {string} sessionId - 会话ID
     * @param {string} operationName - 操作名称
     * @returns {Promise<Object>} - 会话状态
     */
    async ensureActiveSession(sessionId, operationName = '操作') {
        console.log(`[${operationName}] 检查会话 ${sessionId} 状态`);
        const session = this.sessions.get(sessionId);
        
        if (!session) {
            console.error(`[${operationName}] 会话 ${sessionId} 未找到`);
            return { success: false, error: '会话未找到' };
        }

        // 文件管理会话不需要shell流，只需要conn连接
        if (session.isFileManager) {
            if (!session.conn) {
                console.error(`[${operationName}] 文件管理会话 ${sessionId} 缺少连接`);
                return { success: false, error: '文件管理会话缺少连接' };
            }
            // 文件管理会话直接返回，不需要shell流
            return { success: true, session, sessionId };
        }

        // shell 正在异步建立中，调用方可走入队路径，无需重连
        if (!session.stream && session.shellPending) {
            return { success: true, session, sessionId };
        }

        if (!session.stream) {
            console.error(`[${operationName}] 会话 ${sessionId} 的shell未启动`);
            try {
                // 尝试重新激活会话
                const result = await this.activateSession(sessionId);
                if (result.success) {
                    return { 
                        success: true, 
                        session: this.sessions.get(result.sessionId), 
                        sessionId: result.sessionId 
                    };
                }
                return { success: false, error: 'shell未启动且无法重新激活会话' };
            } catch (err) {
                console.error(`[${operationName}] 尝试重新激活会话失败:`, err);
                return { success: false, error: '重新激活会话失败: ' + err.message };
            }
        }

        if (!session.active) {
            console.warn(`[${operationName}] 会话 ${sessionId} 不活跃，重新激活`);
            session.active = true;
            this.sessions.set(sessionId, session);
        }

        return { success: true, session, sessionId };
    }

    /**
     * 向会话发送数据
     * @param {string} sessionId - 会话ID
     * @param {string} data - 发送的数据
     * @returns {Promise<Object>} - 发送结果
     */
    async sendData(sessionId, data) {
        console.log(`[sendData] 向会话 ${sessionId} 发送数据`);
        
        const sessionResult = await this.ensureActiveSession(sessionId, 'sendData');
        if (!sessionResult.success) {
            return { success: false, error: sessionResult.error };
        }
        
        const session = sessionResult.session;

        // 确保data是字符串格式
        const dataStr = typeof data === 'string' ? data : data.toString('utf8');

        // shell 还在异步建立：把数据排队，等就绪后 flush
        if (!session.stream && session.shellPending) {
            session.pendingWrites = session.pendingWrites || [];
            session.pendingWrites.push(dataStr);
            return { success: true };
        }

        try {
            session.stream.write(dataStr);
            return { success: true };
        } catch (err) {
            console.error(`[sendData] 向会话 ${sessionId} 发送数据失败:`, err);
            return { success: false, error: '发送数据失败: ' + err.message };
        }
    }

    /**
     * 激活会话
     * @param {string} sessionId - 会话ID
     * @returns {Promise<Object>} - 激活结果
     */
    async activateSession(sessionId) {
        console.log(`[activateSession] 开始激活会话 ${sessionId}`);
        const session = this.sessions.get(sessionId);
        if (!session) {
            console.error(`[activateSession] 会话 ${sessionId} 未找到`);
            throw new Error('会话未找到');
        }

        // 标记会话为活跃
        session.active = true;
        this.sessions.set(sessionId, session);
        console.log(`[activateSession] 会话 ${sessionId} 已标记为活跃`);

        // 异步 shell 路径下，getSessionBuffer 和 active=true 之间可能落进新数据，
        // 这些数据因 active=false 被吞没；这里把 bufferReadLen 之后的内容补 emit 一次。
        const buf = session.buffer || '';
        if (buf.length > (session.bufferReadLen || 0)) {
            const tail = buf.slice(session.bufferReadLen);
            session.bufferReadLen = buf.length;
            this.sessions.set(sessionId, session);
            this.emit('data', sessionId, tail);
        }

        // 如果存在流（或正在异步建立），直接返回成功
        if (session.stream || session.shellPending) {
            return {success: true, sessionId};
        }
        
        console.warn(`[activateSession] 会话 ${sessionId} 没有可用的stream，尝试重新建立连接`);

        // 如果没有连接详情，无法重连
        if (!session.details) {
            console.error(`[activateSession] 会话 ${sessionId} 没有连接详情，无法重新连接`);
            return {success: false, error: '无法重新连接：缺少连接详情'};
        }

        try {
            // 使用原有的连接详情重新连接
            console.log(`[activateSession] 尝试为会话 ${sessionId} 重新建立连接`);
            const result = await this.connect(session.details);

            // 连接成功后，迁移会话数据
            if (result && result.sessionId) {
                const newSessionId = result.sessionId;
                console.log(`[activateSession] 重新连接成功，新会话ID: ${newSessionId}`);

                // 如果存在连接ID，更新映射关系
                if (session.connectionId) {
                    this.connectionToSession.set(session.connectionId, newSessionId);
                }

                // 复制旧会话的属性到新会话
                const oldSession = this.sessions.get(sessionId);
                const newSession = this.sessions.get(newSessionId);
                if (oldSession && newSession) {
                    // 保留新会话的conn和stream，复制其他属性
                    const conn = newSession.conn;
                    const stream = newSession.stream;
                    Object.assign(newSession, oldSession);
                    newSession.conn = conn;
                    newSession.stream = stream;
                    newSession.active = true;
                    this.sessions.set(newSessionId, newSession);
                }

                // 删除旧会话
                this.sessions.delete(sessionId);

                return {success: true, sessionId: newSessionId};
            }
            return {success: false, error: '重新连接失败'};
        } catch (error) {
            console.error(`[activateSession] 重新连接失败:`, error);
            return {success: false, error: error.message};
        }
    }

    /**
     * 刷新命令提示符
     * @param {string} sessionId - 会话ID
     * @returns {Promise<Object>} - 操作结果
     */
    async refreshPrompt(sessionId) {
        console.log(`[refreshPrompt] 开始刷新会话 ${sessionId} 的命令提示符`);
        
        const sessionResult = await this.ensureActiveSession(sessionId, 'refreshPrompt');
        if (!sessionResult.success) {
            return { success: false, error: sessionResult.error };
        }
        
        const session = sessionResult.session;

        try {
            // 发送一个无输出的echo命令来刷新提示符
            session.stream.write('echo -n ""\r');
            console.log(`[refreshPrompt] 已发送刷新命令到会话 ${sessionId}`);
            return { success: true };
        } catch (err) {
            console.error(`[refreshPrompt] 发送命令失败:`, err);
            return { success: false, error: '发送命令失败: ' + err.message };
        }
    }

    /**
     * 调整终端大小
     * @param {string} sessionId - 会话ID
     * @param {number} cols - 列数
     * @param {number} rows - 行数
     * @returns {Promise<Object>} - 操作结果
     */
    async resize(sessionId, cols, rows) {
        console.log(`[resize] 调整会话 ${sessionId} 的终端大小为 ${cols}x${rows}`);
        
        const sessionResult = await this.ensureActiveSession(sessionId, 'resize');
        if (!sessionResult.success) {
            return { success: false, error: sessionResult.error };
        }
        
        const session = sessionResult.session;

        // shell 还未就绪：仅记录目标尺寸，就绪后应用
        if (!session.stream && session.shellPending) {
            session.pendingResize = { rows, cols };
            return { success: true };
        }

        try {
            session.stream.setWindow(rows, cols, 0, 0);
            return { success: true };
        } catch (err) {
            console.error(`[resize] 调整终端大小失败:`, err);
            return { success: false, error: '调整终端大小失败: ' + err.message };
        }
    }

    /**
     * 执行命令 - 用于终端会话
     * @param {string} sessionId - 会话ID
     * @param {string} command - 命令
     * @returns {Promise<string>} - 执行结果
     */
    async executeCommand(sessionId, command) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'executeCommand');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;
        const connectionKey = session.connectionKey;

        // 检查exec状态缓存，如果已知不可用则直接使用shell
        if (this.execStatus.get(connectionKey) === false) {
            return this.executeCommandThroughShell(sessionId, command);
        }

        // 尝试使用exec，如果失败则使用shell fallback
        try {
            const result = await this.executeWithExecRetry(session, command, 1); // 只重试1次
            // exec成功，标记为可用
            this.execStatus.set(connectionKey, true);
            return result;
        } catch (error) {
            console.warn('Exec不可用，使用shell fallback:', error.message);
            // 标记exec不可用，避免下次重试
            this.execStatus.set(connectionKey, false);
            return this.executeCommandThroughShell(sessionId, command);
        }
    }

    /**
     * 为文件管理创建独立的SSH会话
     * @param {Object} connectionDetails - 连接详情
     * @returns {Promise<string>} - 文件管理会话ID
     */
    async createFileManagerSession(connectionDetails) {
        try {
            // 创建独立的文件管理会话ID
            const fileSessionId = `file_${Date.now()}`;
            
            // 获取或创建连接
            const { conn, connectionObj, isNew, connectionKey, connectOptions } = 
                await this.getOrCreateConnection(connectionDetails);
            
            // 创建文件管理专用会话对象
            this.sessions.set(fileSessionId, {
                conn,
                stream: null,
                details: connectionDetails,
                connectionKey,
                connectionId: connectionDetails.id,
                active: false, // 文件管理会话不需要终端输出
                isFileManager: true, // 标记为文件管理会话
                buffer: ''
            });
            
            // 如果是新连接，需要连接
            if (isNew) {
                await this.setupConnection(conn, connectionObj, connectOptions, connectionKey);
                this.connectionPool.set(connectionKey, connectionObj);
            } else {
                connectionObj.refCount++;
                connectionObj.lastUsed = Date.now();
            }
            
            console.log(`文件管理会话创建成功: ${fileSessionId}`);
            return fileSessionId;
            
        } catch (error) {
            console.error('创建文件管理会话失败:', error);
            throw error;
        }
    }

    /**
     * 为文件管理执行命令 - 使用独立会话
     * @param {string} fileSessionId - 文件管理会话ID
     * @param {string} command - 命令
     * @returns {Promise<string>} - 执行结果
     */
    async executeFileCommand(fileSessionId, command) {
        const sessionResult = await this.ensureActiveSession(fileSessionId, 'executeFileCommand');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;
        const connectionKey = session.connectionKey;

        // 检查exec状态，如果已知不可用则直接抛出错误，不重试
        if (this.execStatus.get(connectionKey) === false) {
            console.log(`连接 ${connectionKey} 的exec已知不可用，跳过重试`);
            throw new Error('exec通道不可用，服务器限制');
        }

        // 对于文件管理会话，只使用exec，不使用shell fallback
        try {
            return await this.executeWithExecRetry(session, command, 1); // 只尝试1次，不重试
        } catch (error) {
            // 标记exec不可用
            this.execStatus.set(connectionKey, false);
            console.warn(`文件管理命令执行失败: ${command}`, error.message);
            throw new Error(`文件操作失败: ${error.message}`);
        }
    }

    /**
     * 使用exec重试执行命令，避免使用用户shell
     * @param {Object} session - 会话对象
     * @param {string} command - 要执行的命令
     * @param {number} retries - 重试次数
     * @returns {Promise<string>} - 命令输出
     */
    async executeWithExecRetry(session, command, retries = 3) {
        return new Promise((resolve, reject) => {
            const tryExec = (attempt) => {
                session.conn.exec(command, (err, stream) => {
                    if (err) {
                        console.warn(`exec命令失败 (尝试 ${attempt}/${retries}):`, err.message);
                        if (attempt < retries) {
                            // 等待一小段时间后重试
                            setTimeout(() => tryExec(attempt + 1), 1000 * attempt);
                            return;
                        } else {
                            // 所有重试都失败了
                            reject(new Error(`命令执行失败，已重试${retries}次: ${err.message}`));
                            return;
                        }
                    }

                    let data = '';
                    stream.on('data', (chunk) => {
                        data += chunk.toString('utf8');
                    });

                    stream.stderr.on('data', (chunk) => {
                        data += chunk.toString('utf8');
                    });

                    stream.on('close', () => {
                        resolve(data);
                    });
                });
            };

            tryExec(1);
        });
    }

    /**
     * 通过现有shell执行命令(exec channel备用方案)
     * @param {string} sessionId - 会话ID
     * @param {string} command - 要执行的命令
     * @param {boolean} silent - 是否静默执行（不显示在终端）
     * @returns {Promise<string>} - 命令输出
     */
    async executeCommandThroughShell(sessionId, command, silent = false) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'executeCommandThroughShell');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;
        const connectionKey = session.connectionKey;
        
        if (!session.stream) {
            throw new Error('Shell会话不可用');
        }

        // 检查当前连接是否有正在执行的命令
        if (this.commandQueue.has(connectionKey)) {
            throw new Error('连接忙碌中，请稍后重试');
        }

        // 标记连接为忙碌
        this.commandQueue.set(connectionKey, true);

        return new Promise((resolve, reject) => {
            let output = '';
            let commandComplete = false;
            let collectingOutput = false;
            let originalDataHandlers = [];
            
            const timeout = setTimeout(() => {
                if (!commandComplete) {
                    commandComplete = true;
                    session.stream.removeListener('data', dataHandler);
                    this.commandQueue.delete(connectionKey); // 清理队列状态
                    reject(new Error('命令执行超时'));
                }
            }, 8000); // 减少到8秒超时

            // 生成唯一标记
            const startMarker = `__CMD_START_${Date.now()}__`;
            const endMarker = `__CMD_END_${Date.now()}__`;
            
            // 使用更简单的方法，避免临时文件
            const fullCommand = `printf "${startMarker}\\n"; ${command}; printf "\\n${endMarker}\\n"`;

            // 监听数据
            const dataHandler = (data) => {
                const chunk = data.toString('utf8');
                
                // 开始收集输出
                if (chunk.includes(startMarker)) {
                    collectingOutput = true;
                    output = '';
                    // 移除开始标记之前的内容
                    const startIndex = chunk.indexOf(startMarker);
                    if (startIndex !== -1) {
                        const afterStart = chunk.substring(startIndex + startMarker.length);
                        if (afterStart.trim()) {
                            output += afterStart;
                        }
                    }
                    return;
                }
                
                // 收集命令输出
                if (collectingOutput) {
                    // 检查是否包含结束标记
                    if (chunk.includes(endMarker)) {
                        commandComplete = true;
                        clearTimeout(timeout);
                        
                        // 添加结束标记之前的内容
                        const endIndex = chunk.indexOf(endMarker);
                        if (endIndex !== -1) {
                            output += chunk.substring(0, endIndex);
                        }
                        
                        // 移除数据处理器
                        session.stream.removeListener('data', dataHandler);
                        
                        // 清理队列状态
                        this.commandQueue.delete(connectionKey);
                        
                        // 清理输出（移除多余的换行和空白）
                        const result = output.trim();
                        resolve(result);
                    } else {
                        output += chunk;
                    }
                } else {
                    // 命令执行前的输出也不要发送到终端
                    // 静默处理
                }
            };

            session.stream.on('data', dataHandler);
            
            // 发送命令
            session.stream.write(fullCommand + '\n');
        });
    }

    /**
     * 暂时拦截数据流，避免发送到终端
     */
    temporarilyInterceptData(stream) {
        // 不移除现有处理器，而是标记拦截状态
        // 这样可以避免破坏现有的数据流处理
        return [];
    }

    /**
     * 恢复原始数据处理器
     */
    restoreDataHandlers(stream, originalHandlers, temporaryHandler) {
        // 移除临时处理器
        stream.removeListener('data', temporaryHandler);
        // 不需要恢复处理器，因为我们没有移除它们
    }

    /**
     * 列出远程文件
     * @param {string} sessionId - 会话ID
     * @param {string} remotePath - 远程路径
     * @returns {Promise<Array>} - 文件列表
     */
    async listFiles(sessionId, remotePath) {
        // 对于文件列表操作，如果传入的是用户会话，获取或创建文件管理会话
        let fileSessionId = sessionId;
        const session = this.sessions.get(sessionId);
        
        if (session && !session.isFileManager) {
            const connectionKey = session.connectionKey;
            // 检查是否已有文件管理会话
            if (this.fileManagerSessions.has(connectionKey)) {
                fileSessionId = this.fileManagerSessions.get(connectionKey);
                console.log(`复用文件管理会话: ${fileSessionId}`);
            } else {
                // 创建新的文件管理会话
                fileSessionId = await this.createFileManagerSession(session.details);
                this.fileManagerSessions.set(connectionKey, fileSessionId);
            }
        }
        
        const sessionResult = await this.ensureActiveSession(fileSessionId, 'listFiles');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const fileSession = sessionResult.session;
        const connectionKey = fileSession.connectionKey;

        // 检查是否已知SFTP不可用，如果是则直接使用SSH命令
        if (this.sftpStatus.get(connectionKey) === false) {
            return this.listFilesWithSSH(fileSessionId, remotePath);
        }

        return new Promise((resolve, reject) => {
            fileSession.conn.sftp((err, sftp) => {
                if (err) {
                    console.warn('SFTP不可用，尝试使用SSH命令:', err.message);
                    // 标记SFTP不可用，避免重复尝试
                    this.sftpStatus.set(connectionKey, false);
                    // 如果SFTP不可用，使用SSH命令作为备用方案
                    this.listFilesWithSSH(fileSessionId, remotePath)
                        .then(resolve)
                        .catch(reject);
                    return;
                }

                // 标记SFTP可用
                this.sftpStatus.set(connectionKey, true);
                
                sftp.readdir(remotePath, async (err, list) => {
                    if (err) {
                        console.warn('SFTP readdir失败，尝试使用SSH命令:', err.message);
                        // 如果SFTP readdir失败，也尝试SSH命令
                        this.listFilesWithSSH(fileSessionId, remotePath)
                            .then(resolve)
                            .catch(reject);
                        return;
                    }

                    try {
                        // 直接使用 readdir 返回的 attrs,避免重复 stat
                        const detailedList = list.map((item) => {
                            const itemPath = remotePath === '/' ? `/${item.filename}` : `${remotePath}/${item.filename}`;

                            return {
                                name: item.filename,
                                fullPath: itemPath,
                                isDirectory: item.attrs.isDirectory(),
                                size: item.attrs.size,
                                modifyTime: new Date(item.attrs.mtime * 1000),
                                permissions: item.attrs.mode,
                                uid: item.attrs.uid,
                                gid: item.attrs.gid,
                                owner: 'unknown',
                                group: 'unknown'
                            };
                        });

                        // 尝试获取用户名和组名
                        this.enrichWithUserInfo(fileSessionId, detailedList)
                            .then(enrichedList => resolve(enrichedList))
                            .catch(() => resolve(detailedList)); // 如果获取用户信息失败，返回基本信息
                            
                    } catch (error) {
                        // 如果获取详细信息失败，返回基本信息
                        resolve(list.map(item => ({
                            name: item.filename,
                            fullPath: `${remotePath}/${item.filename}`,
                            isDirectory: item.attrs.isDirectory(),
                            size: item.attrs.size,
                            modifyTime: new Date(item.attrs.mtime * 1000),
                            permissions: item.attrs.mode,
                            uid: item.attrs.uid || 0,
                            gid: item.attrs.gid || 0,
                            owner: 'unknown',
                            group: 'unknown'
                        })));
                    }
                });
            });
        });
    }

    /**
     * 使用SSH命令列出文件（SFTP备用方案）
     * @param {string} sessionId - 会话ID 
     * @param {string} remotePath - 远程路径
     * @returns {Promise<Array>} - 文件列表
     */
    async listFilesWithSSH(sessionId, remotePath) {
        try {
            // 检查exec状态，如果不可用则通过用户会话获取
            const session = this.sessions.get(sessionId);
            const connectionKey = session?.connectionKey;
            
            if (this.execStatus.get(connectionKey) === false) {
                console.log('exec不可用，通过用户会话获取文件列表');
                return this.listFilesViaUserSession(sessionId, remotePath);
            }
            
            // 使用 ls -la 命令获取详细文件信息，禁用颜色输出。路径用单引号包裹防止注入
            const safePath = shellQuote(remotePath);
            const safePathSlash = shellQuote(remotePath.endsWith('/') ? remotePath : remotePath + '/');
            const command = `LANG=C.UTF-8 LC_ALL=C.UTF-8 ls -la --color=never ${safePath} 2>/dev/null || LANG=C.UTF-8 LC_ALL=C.UTF-8 ls -la --color=never ${safePathSlash} 2>/dev/null`;
            
            let result;
            try {
                result = await this.executeFileCommand(sessionId, command);
            } catch (error) {
                // 如果executeFileCommand失败（通常是exec不可用），切换到用户会话
                console.log('executeFileCommand失败，切换到用户会话');
                return this.listFilesViaUserSession(sessionId, remotePath);
            }
            
            if (!result || result.trim() === '') {
                throw new Error('目录为空或无法访问');
            }
            
            // 清理ANSI转义序列（颜色代码）
            const cleanResult = result.replace(/\x1b\[[0-9;]*m/g, '');
            
            const lines = cleanResult.split('\n').filter(line => line.trim());
            const files = [];
            
            // 跳过第一行（总计行）
            for (let i = 1; i < lines.length; i++) {
                const line = lines[i].trim();
                if (!line) continue;
                
                // 解析 ls -la 输出
                const parts = line.split(/\s+/);
                if (parts.length < 9) continue;
                
                const permissions = parts[0];
                const linkCount = parts[1];
                const owner = parts[2];
                const group = parts[3];
                const size = parseInt(parts[4]) || 0;
                const month = parts[5];
                const day = parts[6];
                const timeOrYear = parts[7];
                const name = parts.slice(8).join(' ');
                
                // 跳过 . 和 .. (如果需要的话)
                if (name === '.' || name === '..') continue;
                
                // 判断是否为目录
                const isDirectory = permissions.startsWith('d');
                
                // 构造文件信息对象
                const fileInfo = {
                    name: name,
                    fullPath: remotePath === '/' ? `/${name}` : `${remotePath}/${name}`,
                    isDirectory: isDirectory,
                    size: isDirectory ? 0 : size,
                    modifyTime: this.parseFileDate(month, day, timeOrYear),
                    permissions: this.convertPermissionsToOctal(permissions),
                    owner: owner,
                    group: group,
                    uid: 0, // SSH命令无法获取UID，设为0
                    gid: 0  // SSH命令无法获取GID，设为0
                };
                
                files.push(fileInfo);
            }
            
            // 检查exec状态，如果不可用则不尝试获取用户信息（避免在终端显示命令）
            if (this.execStatus.get(connectionKey) === false) {
                console.log('exec不可用，跳过用户信息获取，保持UID/GID显示');
                return files;
            }
            
            // 尝试获取用户名和组名
            try {
                const enrichedFiles = await this.enrichWithUserInfo(sessionId, files);
                return enrichedFiles;
            } catch (error) {
                console.warn('获取用户信息失败，返回基本信息:', error.message);
                return files;
            }
            
        } catch (error) {
            console.error('SSH命令列出文件失败:', error);
            throw new Error(`无法访问目录: ${error.message}`);
        }
    }
    
    /**
     * 解析文件日期
     * @param {string} month - 月份
     * @param {string} day - 日期  
     * @param {string} timeOrYear - 时间或年份
     * @returns {Date} - 日期对象
     */
    parseFileDate(month, day, timeOrYear) {
        const months = {
            'Jan': 0, 'Feb': 1, 'Mar': 2, 'Apr': 3, 'May': 4, 'Jun': 5,
            'Jul': 6, 'Aug': 7, 'Sep': 8, 'Oct': 9, 'Nov': 10, 'Dec': 11
        };
        
        const monthNum = months[month] || 0;
        const dayNum = parseInt(day) || 1;
        
        let year, hour = 0, minute = 0;
        
        if (timeOrYear.includes(':')) {
            // 时间格式 (HH:MM)，使用当前年份
            year = new Date().getFullYear();
            const timeParts = timeOrYear.split(':');
            hour = parseInt(timeParts[0]) || 0;
            minute = parseInt(timeParts[1]) || 0;
        } else {
            // 年份格式
            year = parseInt(timeOrYear) || new Date().getFullYear();
        }
        
        return new Date(year, monthNum, dayNum, hour, minute);
    }

    /**
     * 将权限字符串转换为八进制数字
     * @param {string} permStr - 权限字符串（如 'drwxr-xr-x'）
     * @returns {number} - 八进制权限数字
     */
    convertPermissionsToOctal(permStr) {
        if (!permStr || permStr.length < 10) {
            return 0;
        }
        
        // 跳过第一个字符（文件类型）
        const perms = permStr.slice(1);
        let octal = 0;
        
        // 所有者权限 (rwx) - 第7-9位
        let userPerm = 0;
        if (perms[0] === 'r') userPerm += 4;
        if (perms[1] === 'w') userPerm += 2;
        if (perms[2] === 'x' || perms[2] === 's' || perms[2] === 'S') userPerm += 1;
        octal += userPerm * 64; // 左移6位 (8^2)
        
        // 组权限 (rwx) - 第4-6位
        let groupPerm = 0;
        if (perms[3] === 'r') groupPerm += 4;
        if (perms[4] === 'w') groupPerm += 2;
        if (perms[5] === 'x' || perms[5] === 's' || perms[5] === 'S') groupPerm += 1;
        octal += groupPerm * 8; // 左移3位 (8^1)
        
        // 其他用户权限 (rwx) - 第1-3位
        let otherPerm = 0;
        if (perms[6] === 'r') otherPerm += 4;
        if (perms[7] === 'w') otherPerm += 2;
        if (perms[8] === 'x' || perms[8] === 't' || perms[8] === 'T') otherPerm += 1;
        octal += otherPerm; // 不需要左移
        
        // 添加文件类型位
        const fileType = permStr[0];
        if (fileType === 'd') octal += 0o40000; // 目录
        else if (fileType === 'l') octal += 0o120000; // 符号链接
        else if (fileType === 'c') octal += 0o20000; // 字符设备
        else if (fileType === 'b') octal += 0o60000; // 块设备
        else if (fileType === 'p') octal += 0o10000; // 命名管道
        else if (fileType === 's') octal += 0o140000; // 套接字
        else octal += 0o100000; // 普通文件
        
        return octal;
    }

    /**
     * 通过用户会话获取文件列表（当exec不可用时）
     * @param {string} fileSessionId - 文件管理会话ID
     * @param {string} remotePath - 远程路径
     * @returns {Promise<Array>} - 文件列表
     */
    async listFilesViaUserSession(fileSessionId, remotePath) {
        try {
            // 找到对应的用户会话
            const session = this.sessions.get(fileSessionId);
            if (!session) {
                throw new Error('会话不存在');
            }
            
            const userSession = Array.from(this.sessions.values()).find(s => 
                s.connectionKey === session.connectionKey && !s.isFileManager && s.active
            );
            
            if (!userSession) {
                throw new Error('没有可用的用户会话');
            }
            
            // 通过用户会话执行ls命令
            const command = `LANG=C.UTF-8 LC_ALL=C.UTF-8 ls -la --color=never "${remotePath}" 2>/dev/null || LANG=C.UTF-8 LC_ALL=C.UTF-8 ls -la --color=never "${remotePath}/" 2>/dev/null`;
            const userSessionId = Array.from(this.sessions.keys()).find(k => this.sessions.get(k) === userSession);
            const result = await this.executeCommandThroughShell(userSessionId, command, true);
            
            if (!result || result.trim() === '') {
                throw new Error('目录为空或无法访问');
            }
            
            // 清理ANSI转义序列
            const cleanResult = result.replace(/\x1b\[[0-9;]*m/g, '');
            const lines = cleanResult.split('\n').filter(line => line.trim());
            const files = [];
            
            // 跳过第一行（总计行）
            for (let i = 1; i < lines.length; i++) {
                const line = lines[i].trim();
                if (!line) continue;
                
                // 解析 ls -la 输出
                const parts = line.split(/\s+/);
                if (parts.length < 9) continue;
                
                const permissions = parts[0];
                const owner = parts[2];  // 已经是用户名
                const group = parts[3];  // 已经是组名
                const size = parseInt(parts[4]) || 0;
                
                // 获取文件名（处理包含空格的文件名）
                const nameStartIndex = line.indexOf(parts[8]);
                const fileName = line.substring(nameStartIndex);
                
                // 跳过 . 和 .. 目录项
                if (fileName === '.' || fileName === '..') continue;
                
                // 解析日期
                const month = parts[5];
                const day = parts[6];
                const timeOrYear = parts[7];
                const modifyTime = this.parseFileDate(month, day, timeOrYear);
                
                const fileInfo = {
                    name: fileName,
                    fullPath: remotePath === '/' ? `/${fileName}` : `${remotePath}/${fileName}`,
                    isDirectory: permissions.startsWith('d'),
                    size: size,
                    modifyTime: modifyTime,
                    permissions: this.convertPermissionsToOctal(permissions),
                    owner: owner,  // 直接使用ls输出的用户名
                    group: group,  // 直接使用ls输出的组名
                    uid: 0,  // 通过ls无法获取数字UID
                    gid: 0   // 通过ls无法获取数字GID
                };
                
                files.push(fileInfo);
            }
            
            return files;
            
        } catch (error) {
            console.error('通过用户会话获取文件列表失败:', error);
            throw new Error(`无法访问目录: ${error.message}`);
        }
    }

    /**
     * 通过执行系统命令获取用户和组信息
     * @param {string} sessionId - 会话ID
     * @param {Array} fileList - 文件列表
     * @returns {Promise<Array>} - 增强后的文件列表
     */
    async enrichWithUserInfo(sessionId, fileList) {
        try {
            const session = this.sessions.get(sessionId);
            if (!session) {
                return fileList;
            }

            const connectionKey = session.connectionKey;

            // 配置选项: 可通过环境变量 SFTP_DISABLE_USER_LOOKUP=1 关闭用户名/组名查询
            // 直接显示 UID/GID 可以显著提升高延迟链路的性能
            const ENABLE_USER_GROUP_LOOKUP = process.env.SFTP_DISABLE_USER_LOOKUP !== '1';

            if (!ENABLE_USER_GROUP_LOOKUP) {
                console.log('用户名/组名查询已禁用,直接显示UID/GID');

                return fileList.map(file => ({
                    ...file,
                    owner: file.uid?.toString() || 'unknown',
                    group: file.gid?.toString() || 'unknown'
                }));
            }

            // 初始化该连接的缓存(带TTL)
            const CACHE_TTL = 5 * 60 * 1000; // 5分钟过期
            const now = Date.now();

            if (!this.userInfoCache.has(connectionKey)) {
                this.userInfoCache.set(connectionKey, {
                    users: new Map(),
                    groups: new Map(),
                    timestamp: now
                });
            }

            const cache = this.userInfoCache.get(connectionKey);

            // 检查缓存是否过期
            if (now - cache.timestamp > CACHE_TTL) {
                console.log('用户信息缓存已过期,清空:', connectionKey);
                cache.users.clear();
                cache.groups.clear();
                cache.timestamp = now;
            }

            // 获取所有唯一的 UID 和 GID
            const uids = [...new Set(fileList.map(f => f.uid).filter(uid => uid !== undefined))];
            const gids = [...new Set(fileList.map(f => f.gid).filter(gid => gid !== undefined))];

            // 分离已缓存和未缓存的
            const uncachedUids = uids.filter(uid => !cache.users.has(uid));
            const uncachedGids = gids.filter(gid => !cache.groups.has(gid));

            const userMap = new Map(cache.users);
            const groupMap = new Map(cache.groups);
            
            // 如果是文件管理会话且exec已知不可用，或者任何会话的exec状态为false，直接使用用户终端会话
            if (session && (
                (session.isFileManager && this.execStatus.get(session.connectionKey) === false) ||
                this.execStatus.get(session.connectionKey) === false
            )) {
                // 寻找同一连接的用户会话
                const userSession = Array.from(this.sessions.values()).find(s => 
                    s.connectionKey === session.connectionKey && !s.isFileManager && s.active
                );
                
                if (userSession) {
                    console.log('exec不可用，直接使用用户会话获取用户信息');
                    return this.getUserInfoFromUserSession(userSession, fileList);
                }
                
                // 如果没有用户会话，直接返回带有UID/GID的列表
                console.log('没有可用的用户会话，保持UID/GID显示');
                return fileList.map(file => ({
                    ...file,
                    owner: file.uid?.toString() || 'unknown',
                    group: file.gid?.toString() || 'unknown'
                }));
            }
            
            // 只获取未缓存的用户名
            if (uncachedUids.length > 0) {
                try {
                    const userResult = await this.executeFileCommand(sessionId, `getent passwd ${uncachedUids.join(' ')} 2>/dev/null || true`);
                    const userLines = userResult.split('\n').filter(line => line.trim());
                    userLines.forEach(line => {
                        const parts = line.split(':');
                        if (parts.length >= 3) {
                            const uid = parseInt(parts[2]);
                            const username = parts[0];
                            userMap.set(uid, username);
                            cache.users.set(uid, username); // 更新缓存
                        }
                    });
                } catch (err) {
                    console.warn('获取用户信息失败:', err.message);
                }
            }

            // 只获取未缓存的组名
            if (uncachedGids.length > 0) {
                try {
                    const groupResult = await this.executeFileCommand(sessionId, `getent group ${uncachedGids.join(' ')} 2>/dev/null || true`);
                    const groupLines = groupResult.split('\n').filter(line => line.trim());
                    groupLines.forEach(line => {
                        const parts = line.split(':');
                        if (parts.length >= 3) {
                            const gid = parseInt(parts[2]);
                            const groupname = parts[0];
                            groupMap.set(gid, groupname);
                            cache.groups.set(gid, groupname); // 更新缓存
                        }
                    });
                } catch (err) {
                    console.warn('获取组信息失败:', err.message);
                }
            }
            
            // 应用用户和组信息
            return fileList.map(file => ({
                ...file,
                owner: userMap.get(file.uid) || file.uid?.toString() || 'unknown',
                group: groupMap.get(file.gid) || file.gid?.toString() || 'unknown'
            }));
            
        } catch (error) {
            console.warn('增强用户信息失败:', error);
            return fileList;
        }
    }

    /**
     * 通过用户终端会话获取用户信息
     * @param {Object} userSession - 用户会话对象
     * @param {Array} fileList - 文件列表
     * @returns {Promise<Array>} - 增强后的文件列表
     */
    async getUserInfoFromUserSession(userSession, fileList) {
        try {
            const userMap = new Map();
            const groupMap = new Map();
            
            // 获取所有唯一的 UID 和 GID
            const uids = [...new Set(fileList.map(f => f.uid).filter(uid => uid !== undefined))];
            const gids = [...new Set(fileList.map(f => f.gid).filter(gid => gid !== undefined))];
            
            // 获取用户会话的sessionId
            const userSessionId = Array.from(this.sessions.keys()).find(k => this.sessions.get(k) === userSession);
            if (!userSessionId) {
                throw new Error('无法找到用户会话ID');
            }
            
            // 通过用户会话的shell执行命令获取用户信息
            if (uids.length > 0) {
                try {
                    // 确保UIDs是有效的数字
                    const validUids = uids.filter(uid => uid != null && !isNaN(uid) && uid >= 0);
                    if (validUids.length === 0) {
                        console.log('没有有效的UID，跳过用户信息获取');
                    } else {
                        const userResult = await this.executeCommandThroughShell(userSessionId, `getent passwd ${validUids.join(' ')} 2>/dev/null || true`);
                        const userLines = userResult.split('\n').filter(line => line.trim());
                        userLines.forEach(line => {
                            const parts = line.split(':');
                            if (parts.length >= 3) {
                                userMap.set(parseInt(parts[2]), parts[0]);
                            }
                        });
                    }
                } catch (err) {
                    console.warn('通过用户会话获取用户信息失败:', err.message);
                }
            }
            
            if (gids.length > 0) {
                try {
                    // 确保GIDs是有效的数字
                    const validGids = gids.filter(gid => gid != null && !isNaN(gid) && gid >= 0);
                    if (validGids.length === 0) {
                        console.log('没有有效的GID，跳过组信息获取');
                    } else {
                        const groupResult = await this.executeCommandThroughShell(userSessionId, `getent group ${validGids.join(' ')} 2>/dev/null || true`);
                        const groupLines = groupResult.split('\n').filter(line => line.trim());
                        groupLines.forEach(line => {
                            const parts = line.split(':');
                            if (parts.length >= 3) {
                                groupMap.set(parseInt(parts[2]), parts[0]);
                            }
                        });
                    }
                } catch (err) {
                    console.warn('通过用户会话获取组信息失败:', err.message);
                }
            }
            
            // 应用用户和组信息
            return fileList.map(file => ({
                ...file,
                owner: userMap.get(file.uid) || file.uid?.toString() || 'unknown',
                group: groupMap.get(file.gid) || file.gid?.toString() || 'unknown'
            }));
            
        } catch (error) {
            console.warn('通过用户会话获取用户信息失败:', error);
            return fileList.map(file => ({
                ...file,
                owner: file.uid?.toString() || 'unknown',
                group: file.gid?.toString() || 'unknown'
            }));
        }
    }

    /**
     * 上传文件
     * @param {string} sessionId - 会话ID
     * @param {string} localPath - 本地路径
     * @param {string} remotePath - 远程路径
     * @returns {Promise<void>}
     */
    async uploadFile(sessionId, localPath, remotePath) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'uploadFile');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;

        return new Promise((resolve, reject) => {
            session.conn.sftp((err, sftp) => {
                if (err) {
                    reject(err);
                    return;
                }

                // 使用选项优化传输性能
                const transferOptions = {
                    concurrency: 64, // 增加并发数
                    chunkSize: 32768, // 32KB chunks
                    step: (total_transferred, chunk, total) => {
                        // 发送进度事件
                        const progress = Math.round((total_transferred / total) * 100);
                        this.emit('transfer-progress', sessionId, {
                            type: 'upload',
                            path: remotePath,
                            progress,
                            transferred: total_transferred,
                            total
                        });
                    }
                };
                
                sftp.fastPut(localPath, remotePath, transferOptions, (err) => {
                    if (err) {
                        reject(err);
                        return;
                    }
                    resolve();
                });
            });
        });
    }

    /**
     * 创建远程目录
     * @param {string} sessionId - 会话ID
     * @param {string} remotePath - 远程路径
     * @returns {Promise<void>}
     */
    async createDirectory(sessionId, remotePath) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'createDirectory');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;

        return new Promise((resolve, reject) => {
            session.conn.sftp((err, sftp) => {
                if (err) {
                    reject(err);
                    return;
                }

                sftp.mkdir(remotePath, (err) => {
                    if (err) {
                        reject(err);
                        return;
                    }
                    resolve();
                });
            });
        });
    }

    /**
     * 上传目录
     * @param {string} sessionId - 会话ID
     * @param {string} localPath - 本地路径
     * @param {string} remotePath - 远程路径
     * @returns {Promise<boolean>}
     */
    async uploadDirectory(sessionId, localPath, remotePath) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'uploadDirectory');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;

        // Get SFTP instance once for the entire operation
        const sftp = await new Promise((resolve, reject) => {
            session.conn.sftp((err, sftp) => {
                if (err) reject(err);
                else resolve(sftp);
            });
        });

        // Create remote directory
        try {
            await new Promise((resolve, reject) => {
                sftp.mkdir(remotePath, err => {
                    // Ignore if directory already exists (code 4)
                    if (err && err.code !== 4) {
                        console.warn(`Warning creating dir ${remotePath}:`, err);
                    }
                    resolve(); // Continue anyway
                });
            });
        } catch (error) {
            console.warn(`Warning creating base dir:`, error);
            // Continue regardless of error - directory may exist
        }

        // Process files and directories
        const processItem = async (localItemPath, remoteItemPath) => {
            const stats = fs.statSync(localItemPath);

            if (stats.isDirectory()) {
                // Create directory on remote
                try {
                    await new Promise((resolve, reject) => {
                        sftp.mkdir(remoteItemPath, err => {
                            // Ignore if directory already exists
                            if (err && err.code !== 4) {
                                console.warn(`Warning creating dir ${remoteItemPath}:`, err);
                            }
                            resolve(); // Continue anyway
                        });
                    });
                } catch (error) {
                    console.warn(`Warning creating dir:`, error);
                    // Continue regardless of error
                }

                // Process all items in directory
                const items = fs.readdirSync(localItemPath);
                for (const item of items) {
                    await processItem(
                        path.join(localItemPath, item),
                        `${remoteItemPath}/${item}`
                    );
                }
            } else {
                // Upload file
                await new Promise((resolve, reject) => {
                    sftp.fastPut(localItemPath, remoteItemPath, err => {
                        if (err) reject(err);
                        else resolve();
                    });
                });
            }
        };

        // Start recursive upload operation
        await processItem(localPath, remotePath);
        return true;
    }

    /**
     * 下载文件
     * @param {string} sessionId - 会话ID
     * @param {string} remotePath - 远程路径
     * @param {string} localPath - 本地路径
     * @returns {Promise<void>}
     */
    async downloadFile(sessionId, remotePath, localPath) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'downloadFile');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;

        return new Promise((resolve, reject) => {
            session.conn.sftp((err, sftp) => {
                if (err) {
                    reject(err);
                    return;
                }

                // Create parent directory if it doesn't exist
                const parentDir = path.dirname(localPath);
                if (!fs.existsSync(parentDir)) {
                    fs.mkdirSync(parentDir, {recursive: true});
                }

                // Use fastGet to download the file
                sftp.fastGet(remotePath, localPath, (err) => {
                    if (err) {
                        reject(err);
                        return;
                    }
                    resolve();
                });
            });
        });
    }

    /**
     * 下载目录
     * @param {string} sessionId - 会话ID 
     * @param {string} remotePath - 远程路径
     * @param {string} localPath - 本地路径
     * @returns {Promise<boolean>}
     */
    async downloadDirectory(sessionId, remotePath, localPath) {
        const sessionResult = await this.ensureActiveSession(sessionId, 'downloadDirectory');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }
        
        const session = sessionResult.session;

        // 创建本地目录
        if (!fs.existsSync(localPath)) {
            fs.mkdirSync(localPath, { recursive: true });
        }

        // 获取SFTP
        const sftp = await new Promise((resolve, reject) => {
            session.conn.sftp((err, sftp) => {
                if (err) reject(err);
                else resolve(sftp);
            });
        });

        // 读取远程目录
        const list = await new Promise((resolve, reject) => {
            sftp.readdir(remotePath, (err, list) => {
                if (err) reject(err);
                else resolve(list);
            });
        });

        // 处理目录内容
        for (const item of list) {
            const remoteItemPath = `${remotePath}/${item.filename}`;
            const localItemPath = path.join(localPath, item.filename);

            if (item.attrs.isDirectory()) {
                // 如果是目录，递归下载
                await this.downloadDirectory(sessionId, remoteItemPath, localItemPath);
            } else {
                // 如果是文件，下载文件
                await new Promise((resolve, reject) => {
                    sftp.fastGet(remoteItemPath, localItemPath, (err) => {
                        if (err) reject(err);
                        else {
                            this.emit('download-progress', {
                                sessionId,
                                file: item.filename,
                                current: item.attrs.size,
                                total: item.attrs.size,
                                percent: 100
                            });
                            resolve();
                        }
                    });
                });
            }
        }

        return true;
    }

    /**
     * 获取会话缓冲区
     * @param {string} sessionId - 会话ID
     * @returns {Promise<Object>} - 缓冲区内容
     */
    async getSessionBuffer(sessionId) {
        const session = this.sessions.get(sessionId);
        if (!session) {
            return {success: false, error: '会话未找到'};
        }

        const buffer = session.buffer || '';
        // 记录 renderer 已读到的位置；activateSession 会把这之后到达但未 emit 的数据补 emit。
        session.bufferReadLen = buffer.length;
        this.sessions.set(sessionId, session);

        return {
            success: true,
            buffer
        };
    }

    /**
     * 修改文件权限
     * @param {string} sessionId - 会话ID
     * @param {string} remotePath - 远程文件路径
     * @param {string} permissions - 权限（八进制字符串，如 '755'）
     * @returns {Promise<boolean>}
     */
    async changeFilePermissions(sessionId, remotePath, permissions) {
        if (!isValidOctalPermissions(permissions)) {
            throw new Error('权限格式不合法，必须为 3-4 位八进制数字');
        }
        if (typeof remotePath !== 'string' || !remotePath) {
            throw new Error('远程路径不能为空');
        }

        const sessionResult = await this.ensureActiveSession(sessionId, 'changeFilePermissions');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }

        try {
            // 使用 chmod 命令修改权限，路径用单引号安全包裹防止注入
            const command = `chmod ${permissions} ${shellQuote(remotePath)}`;
            await this.executeCommand(sessionId, command);
            return true;
        } catch (error) {
            console.error('修改文件权限失败:', error);
            throw new Error(`修改文件权限失败: ${error.message}`);
        }
    }

    /**
     * 修改文件所有者
     * @param {string} sessionId - 会话ID
     * @param {string} remotePath - 远程文件路径
     * @param {string} owner - 新的所有者
     * @param {string} group - 新的组（可选）
     * @returns {Promise<boolean>}
     */
    async changeFileOwner(sessionId, remotePath, owner, group = null) {
        if (!isValidUnixName(owner)) {
            throw new Error('所有者名称不合法');
        }
        if (group && !isValidUnixName(group)) {
            throw new Error('组名称不合法');
        }
        if (typeof remotePath !== 'string' || !remotePath) {
            throw new Error('远程路径不能为空');
        }

        const sessionResult = await this.ensureActiveSession(sessionId, 'changeFileOwner');
        if (!sessionResult.success) {
            throw new Error(sessionResult.error);
        }

        try {
            // 校验过的 owner/group + 单引号包裹路径，防注入
            const ownerGroup = group ? `${owner}:${group}` : owner;
            const command = `chown ${ownerGroup} ${shellQuote(remotePath)}`;
            await this.executeCommand(sessionId, command);
            return true;
        } catch (error) {
            console.error('修改文件所有者失败:', error);
            throw new Error(`修改文件所有者失败: ${error.message}`);
        }
    }
}

// 导出单例
module.exports = new SshService();
