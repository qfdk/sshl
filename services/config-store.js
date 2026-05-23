const Store = require('electron-store');
const path = require('path');
const fs = require('fs');
const os = require('os');
const crypto = require('crypto');

class ConfigStore {
  constructor() {
    // Create ~/.sshl directory if it doesn't exist
    this.sshlDir = path.join(os.homedir(), '.sshl');
    if (!fs.existsSync(this.sshlDir)) {
      fs.mkdirSync(this.sshlDir, { mode: 0o700, recursive: true }); // Set permissions to only allow owner access
    }

    // Main configuration store (non-sensitive data)
    this.store = new Store({
      name: 'ssh-client-config',
      cwd: this.sshlDir,
      defaults: {
        connections: []
      }
    });

    // Sensitive data store (passwords, keys)
    this.sensitiveStore = new Store({
      name: 'credentials',
      cwd: this.sshlDir, // Store in ~/.sshl directory
      encryptionKey: 'sshl-secure-credentials', // Basic encryption
      defaults: {
        credentials: {}
      }
    });
  }

  getConnections() {
    const connections = this.store.get('connections') || [];

    // Merge with sensitive data when requested
    return connections.map(conn => {
      const id = conn.id;
      const credentials = this.sensitiveStore.get(`credentials.${id}`);

      if (credentials) {
        // Merge credentials with connection info
        return {
          ...conn,
          password: credentials.password || '',
          passphrase: credentials.passphrase || ''
        };
      }

      return conn;
    });
  }

  saveConnection(connection) {
    const connections = this.store.get('connections') || [];

    // Separate sensitive data
    const sensitiveData = {
      password: connection.password || '',
      passphrase: connection.passphrase || ''
    };

    // Create connection without sensitive data
    const cleanConnection = { ...connection };
    delete cleanConnection.password;
    delete cleanConnection.passphrase;

    // If the connection has an ID, update it
    if (connection.id) {
      const index = connections.findIndex(c => c.id === connection.id);
      if (index !== -1) {
        connections[index] = cleanConnection;
      } else {
        connections.push(cleanConnection);
      }

      // Save sensitive data separately
      this.sensitiveStore.set(`credentials.${connection.id}`, sensitiveData);

    } else {
      // New connection - assign an ID
      cleanConnection.id = crypto.randomUUID();
      connections.push(cleanConnection);

      // Save sensitive data separately
      this.sensitiveStore.set(`credentials.${cleanConnection.id}`, sensitiveData);
    }

    this.store.set('connections', connections);
    return connection;
  }

  deleteConnection(id) {
    if (!id) {
      console.error('无效的连接ID');
      return false;
    }

    try {
      const connections = this.getConnections();
      if (!connections || !Array.isArray(connections)) {
        console.error('无法获取连接列表');
        return false;
      }

      const filtered = connections.filter(c => c && c.id !== id);
      this.store.set('connections', filtered);

      // Also delete sensitive data
      this.sensitiveStore.delete(`credentials.${id}`);

      return true;
    } catch (error) {
      console.error('删除连接失败:', error);
      return false;
    }
  }
}

module.exports = ConfigStore;
