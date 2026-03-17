<template>
  <div class="page">
    <div class="page-header">
      <h1>Node Status</h1>
      <div class="header-controls">
        <label class="auto-refresh-toggle">
          <input type="checkbox" v-model="autoRefresh" @change="toggleAuto" />
          <span>Auto refresh ({{ refreshInterval / 1000 }}s)</span>
        </label>
        <button class="refresh-btn" @click="fetchStatus">↻ Refresh</button>
      </div>
    </div>

    <div v-if="loading" class="empty"><span class="spin">⟳</span> Connecting to node...</div>

    <div v-else-if="error" class="error-card">
      <div class="error-icon">✕</div>
      <h2>Node Unreachable</h2>
      <p>{{ error }}</p>
      <button class="back-btn" @click="fetchStatus">Retry</button>
    </div>

    <div v-else class="grid">
      <!-- Mining -->
      <div class="card" :class="{ 'card-active': status.mining?.active }">
        <div class="card-header">
          <span class="card-title">Mining</span>
          <span class="status-dot" :class="status.mining?.active ? 'online' : 'offline'">
            {{ status.mining?.active ? 'Mining' : 'Idle' }}
          </span>
        </div>
        <div class="info-grid">
          <div class="info-item">
            <span class="info-label">Hashrate</span>
            <span class="info-val accent-text">{{ formatHashrate(status.mining?.hashrate) }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Difficulty</span>
            <span class="info-val">{{ status.mining?.difficulty || status.blockchain?.difficulty || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Blocks Mined</span>
            <span class="info-val">{{ status.mining?.blocks_mined || 0 }}</span>
          </div>
        </div>
      </div>

      <!-- Blockchain -->
      <div class="card">
        <div class="card-header">
          <span class="card-title">Blockchain</span>
        </div>
        <div class="info-grid">
          <div class="info-item">
            <span class="info-label">Height</span>
            <span class="info-val accent-text bold">{{ status.blockchain?.height || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Difficulty</span>
            <span class="info-val">{{ status.blockchain?.difficulty || 1 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Memory Blocks</span>
            <span class="info-val">{{ status.blockchain?.memory_blocks || 0 }}</span>
          </div>
          <div class="info-item full">
            <span class="info-label">Chain Tip</span>
            <span class="info-val mono text2">{{ formatHash(status.blockchain?.chain_tip) }}</span>
          </div>
        </div>
      </div>

      <!-- Wallet -->
      <div class="card card-wallet">
        <div class="card-header">
          <span class="card-title">Wallet</span>
        </div>
        <div class="info-grid">
          <div class="info-item full">
            <span class="info-label">Address</span>
            <span class="info-val mono accent-text">{{ status.wallet?.address || 'N/A' }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Balance</span>
            <span class="info-val yellow-text bold">{{ formatBalance(status.wallet?.balance) }}</span>
          </div>
        </div>
      </div>

      <!-- Mempool -->
      <div class="card">
        <div class="card-header">
          <span class="card-title">Mempool</span>
        </div>
        <div class="info-grid">
          <div class="info-item">
            <span class="info-label">Pending Txs</span>
            <span class="info-val accent-text bold">{{ status.mempool?.pending_transactions || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Seen Txs</span>
            <span class="info-val">{{ status.mempool?.seen_transactions || 0 }}</span>
          </div>
        </div>
      </div>

      <!-- Network -->
      <div class="card">
        <div class="card-header">
          <span class="card-title">Network</span>
          <span class="peer-count">{{ status.network?.connected_peers || 0 }} peers</span>
        </div>
        <div v-if="peerHeights.length" class="peer-list">
          <div v-for="peer in peerHeights" :key="peer.id" class="peer-row">
            <span class="peer-id mono">{{ peer.id }}</span>
            <span class="peer-height">Block #{{ peer.height }}</span>
          </div>
        </div>
        <div v-else class="empty-peers">No connected peers</div>
      </div>

      <!-- Node Info -->
      <div class="card">
        <div class="card-header">
          <span class="card-title">Node</span>
          <span class="status-dot online">Online</span>
        </div>
        <div class="info-grid">
          <div class="info-item">
            <span class="info-label">Version</span>
            <span class="info-val">{{ status.node?.version || 'N/A' }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Uptime</span>
            <span class="info-val">{{ formatUptime(status.node?.uptime_seconds) }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">Last Update</span>
            <span class="info-val text2 small">{{ formatTime(status.timestamp) }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, onMounted, onUnmounted, computed } from "vue";
import { explorerAPI } from "../api/explorer";
export default {
  name: "NodeStatus",
  setup() {
    const status = ref(null);
    const loading = ref(true);
    const error = ref(null);
    const autoRefresh = ref(true);
    const refreshInterval = ref(5000);
    let timer = null;

    const peerHeights = computed(() => {
      if (!status.value?.network?.peer_heights) return [];
      return Object.entries(status.value.network.peer_heights).map(([id, height]) => ({ id, height }));
    });

    const fetchStatus = async () => {
      try {
        loading.value = true;
        error.value = null;
        const res = await explorerAPI.getNodeStatus();
        status.value = res.data;
      } catch (e) {
        error.value = e.response?.data?.message || "Unable to reach node.";
      } finally { loading.value = false; }
    };

    const startAuto = () => { stopAuto(); timer = setInterval(fetchStatus, refreshInterval.value); };
    const stopAuto = () => { if (timer) { clearInterval(timer); timer = null; } };
    const toggleAuto = () => autoRefresh.value ? startAuto() : stopAuto();

    const formatHashrate = (h) => {
      if (!h) return "0 H/s";
      if (h >= 1e12) return `${(h/1e12).toFixed(2)} TH/s`;
      if (h >= 1e9) return `${(h/1e9).toFixed(2)} GH/s`;
      if (h >= 1e6) return `${(h/1e6).toFixed(2)} MH/s`;
      if (h >= 1e3) return `${(h/1e3).toFixed(2)} KH/s`;
      return `${h.toFixed(2)} H/s`;
    };

    const formatHash = (hash) => {
      if (!hash || hash === "none") return "N/A";
      return hash.length > 16 ? hash.slice(0, 10) + "…" + hash.slice(-8) : hash;
    };

    const formatUptime = (s) => {
      if (!s) return "N/A";
      const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
      return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m ${sec}s` : `${sec}s`;
    };

    const formatTime = (ts) => ts ? new Date(ts).toLocaleString("ko-KR") : "N/A";

    const formatBalance = (hex) => {
      if (!hex) return "0 ASRM";
      try {
        const h = hex.startsWith("0x") ? hex.slice(2) : hex;
        return (Number(BigInt("0x" + h)) / 1e18).toFixed(4) + " ASRM";
      } catch { return "0 ASRM"; }
    };

    onMounted(() => { fetchStatus(); if (autoRefresh.value) startAuto(); });
    onUnmounted(() => stopAuto());

    return { status, loading, error, autoRefresh, refreshInterval, peerHeights, fetchStatus, toggleAuto, formatHashrate, formatHash, formatUptime, formatTime, formatBalance };
  },
};
</script>

<style scoped>
.page { width: 100%; }

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

h1 { font-size: 1.5rem; font-weight: 700; color: var(--text); }

.header-controls {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.auto-refresh-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text2);
  font-size: 12px;
  cursor: pointer;
}

.auto-refresh-toggle input { cursor: pointer; accent-color: var(--accent); }

.refresh-btn {
  padding: 0.4rem 1rem;
  background: var(--surface2);
  border: 1px solid var(--border2);
  border-radius: var(--radius);
  color: var(--text2);
  cursor: pointer;
  font-size: 13px;
  transition: all 0.2s;
}
.refresh-btn:hover { border-color: var(--accent); color: var(--accent); }

.back-btn {
  padding: 0.4rem 1rem;
  background: var(--accent);
  border: none;
  border-radius: var(--radius);
  color: #fff;
  cursor: pointer;
  font-size: 13px;
  font-weight: 600;
  margin-top: 0.5rem;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
  gap: 1.25rem;
}

.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  transition: border-color 0.2s;
}
.card:hover { border-color: var(--border2); }
.card-active { border-color: rgba(16,185,129,.3); background: rgba(16,185,129,.03); }
.card-wallet { border-color: rgba(245,158,11,.2); background: rgba(245,158,11,.03); }

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.status-dot {
  font-size: 11px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 100px;
}
.status-dot.online { background: rgba(16,185,129,.12); color: var(--green); }
.status-dot.offline { background: rgba(100,116,139,.1); color: var(--muted); }

.peer-count {
  font-size: 11px;
  background: rgba(59,130,246,.1);
  color: var(--accent);
  padding: 2px 8px;
  border-radius: 100px;
}

.info-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
  gap: 1px;
  background: var(--border);
  margin: 0;
}

.info-item {
  background: var(--surface);
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.card-active .info-item { background: rgba(16,185,129,.03); }
.card-wallet .info-item { background: rgba(245,158,11,.03); }

.info-item.full { grid-column: 1 / -1; }

.info-label {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
}

.info-val {
  font-size: 1.1rem;
  font-weight: 600;
  color: var(--text);
  word-break: break-all;
}

.info-val.small { font-size: 0.85rem; }

.mono { font-family: var(--mono); font-size: 12px; }
.accent-text { color: var(--accent); }
.yellow-text { color: var(--yellow); }
.text2 { color: var(--text2); }
.bold { font-weight: 700; }

.peer-list { padding: 0.5rem 0; }

.peer-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.6rem 1.25rem;
  border-bottom: 1px solid var(--border);
  transition: background 0.15s;
}
.peer-row:last-child { border-bottom: none; }
.peer-row:hover { background: var(--surface2); }

.peer-id { font-size: 12px; color: var(--text2); font-family: var(--mono); }
.peer-height {
  font-size: 12px;
  font-weight: 600;
  color: var(--accent);
  background: rgba(59,130,246,.1);
  padding: 2px 8px;
  border-radius: 6px;
}

.empty-peers {
  padding: 1.5rem;
  text-align: center;
  color: var(--muted);
  font-size: 12px;
}

.error-card {
  background: var(--surface);
  border: 1px solid rgba(239,68,68,.3);
  border-radius: var(--radius-lg);
  padding: 3rem;
  text-align: center;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
}
.error-icon {
  width: 48px; height: 48px;
  background: rgba(239,68,68,.12);
  color: var(--red);
  border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  font-size: 20px; font-weight: 700;
}
.error-card h2 { color: var(--red); font-size: 1.2rem; }
.error-card p { color: var(--text2); font-size: 13px; }

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}
.spin { display: inline-block; animation: spin 1.5s linear infinite; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 768px) {
  .grid { grid-template-columns: 1fr; }
}
</style>
