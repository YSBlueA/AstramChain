<template>
  <div class="home">
    <!-- Hero -->
    <div class="hero">
      <div class="hero-bg"></div>
      <div class="hero-grid"></div>
      <div class="hero-content">
        <div class="hero-badge">
          <span class="badge-dot"></span>
          Astram Blockchain — Blake3-DAG Proof of Work
        </div>
        <h1>Astram <span class="hero-accent">Explorer</span></h1>
        <p class="hero-sub">Real-time blockchain monitoring for the Astram network</p>
        <div class="search-box">
          <input
            v-model="searchQuery"
            type="text"
            placeholder="Search block height, tx hash, or address..."
            @keyup.enter="handleSearch"
          />
          <button @click="handleSearch">Search</button>
        </div>
      </div>
    </div>

    <!-- Stats Grid -->
    <div v-if="stats" class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Total Blocks</div>
        <div class="stat-value">{{ stats.total_blocks.toLocaleString() }}</div>
        <div class="stat-sub">confirmed blocks</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Transactions</div>
        <div class="stat-value">{{ stats.total_transactions.toLocaleString() }}</div>
        <div class="stat-sub">all time</div>
      </div>
      <div class="stat-card accent-card">
        <div class="stat-label">Network Hashrate</div>
        <div class="stat-value">{{ stats.network_hashrate }}</div>
        <div class="stat-sub">estimated from chain</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Difficulty</div>
        <div class="stat-value">{{ stats.current_difficulty }}</div>
        <div class="stat-sub">leading zeros</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Total Addresses</div>
        <div class="stat-value">{{ stats.total_addresses.toLocaleString() }}</div>
        <div class="stat-sub">unique wallets</div>
      </div>
      <div class="stat-card green-card">
        <div class="stat-label">Circulating Supply</div>
        <div class="stat-value">{{ formatVolumeAmount(stats.circulating_supply) }}</div>
        <div class="stat-sub">ASRM mined</div>
      </div>
    </div>
    <div v-else class="stats-placeholder">
      <div class="stat-skeleton" v-for="i in 6" :key="i"></div>
    </div>

    <!-- Recent Blocks & Transactions -->
    <div class="recent-grid">
      <!-- Recent Blocks -->
      <div class="panel">
        <div class="panel-header">
          <h2>Recent Blocks</h2>
          <router-link to="/blocks" class="panel-link">View all →</router-link>
        </div>
        <div v-if="recentBlocks.length" class="list">
          <div
            v-for="block in recentBlocks.slice(0, 6)"
            :key="block.hash"
            class="list-row"
            @click="$router.push(`/blocks/${block.height}`)"
          >
            <div class="row-icon block-icon">⬛</div>
            <div class="row-main">
              <span class="row-title accent-text">#{{ block.height }}</span>
              <span class="row-sub">{{ truncateHash(block.hash) }}</span>
            </div>
            <div class="row-right">
              <span class="row-badge">{{ block.transactions }} txs</span>
              <span class="row-time">{{ timeAgo(block.timestamp) }}</span>
            </div>
          </div>
        </div>
        <div v-else class="panel-empty">
          <span class="spin">⟳</span> Loading blocks...
        </div>
      </div>

      <!-- Recent Transactions -->
      <div class="panel">
        <div class="panel-header">
          <h2>Recent Transactions</h2>
          <router-link to="/transactions" class="panel-link">View all →</router-link>
        </div>
        <div v-if="recentTransactions.length" class="list">
          <div
            v-for="tx in recentTransactions.slice(0, 6)"
            :key="tx.hash"
            class="list-row"
            @click="$router.push(`/transactions/${tx.hash}`)"
          >
            <div class="row-icon" :class="tx.from === 'Block_Reward' ? 'mining-icon' : 'tx-icon'">
              {{ tx.from === 'Block_Reward' ? '⛏' : '⇄' }}
            </div>
            <div class="row-main">
              <span class="row-title accent-text">{{ truncateHash(tx.hash) }}</span>
              <span class="row-sub">
                <span v-if="tx.from === 'Block_Reward'" class="badge-mining">Mining Reward</span>
                <span v-else>{{ truncateAddr(tx.from) }} → {{ truncateAddr(tx.to) }}</span>
              </span>
            </div>
            <div class="row-right">
              <span class="row-amount green-text">{{ formatAmount(tx.amount) }} ASRM</span>
              <span class="row-time">{{ timeAgo(tx.timestamp) }}</span>
            </div>
          </div>
        </div>
        <div v-else class="panel-empty">
          <span class="spin">⟳</span> Loading transactions...
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "Home",
  data() {
    return {
      searchQuery: "",
      stats: null,
      recentBlocks: [],
      recentTransactions: [],
    };
  },
  mounted() {
    this.fetchData();
    setInterval(() => this.fetchData(), 10000);
  },
  methods: {
    async fetchData() {
      try {
        const [statsRes, blocksRes, txsRes] = await Promise.all([
          explorerAPI.getStats(),
          explorerAPI.getBlocks(1, 10),
          explorerAPI.getTransactions(1, 10),
        ]);
        this.stats = statsRes.data;
        this.recentBlocks = blocksRes.data.blocks || [];
        this.recentTransactions = txsRes.data.transactions || [];
      } catch (e) {
        console.error("Failed to load data:", e);
      }
    },
    async handleSearch() {
      const query = this.searchQuery.trim();
      if (!query) return;
      if (/^\d+$/.test(query)) { this.$router.push(`/blocks/${query}`); return; }
      const norm = query.startsWith("0x") ? query.slice(2) : query;
      const isHex64 = /^[A-Fa-f0-9]{64}$/.test(norm);
      if (isHex64) {
        try { await explorerAPI.getBlockByHash(query); this.$router.push(`/blocks/${query}`); return; } catch {}
        try { await explorerAPI.getTransactionByHash(query); this.$router.push(`/transactions/${query}`); return; } catch {}
        this.$router.push(`/transactions/${query}`); return;
      }
      this.$router.push(`/address/${query}`);
    },
    formatAmount(value) {
      try {
        let num = typeof value === "string" ? BigInt(value) : BigInt(value || 0);
        const d = BigInt("1000000000000000000");
        return (Number(num) / Number(d)).toLocaleString("en-US", { minimumFractionDigits: 0, maximumFractionDigits: 4 });
      } catch { return "0"; }
    },
    formatVolumeAmount(value) {
      try {
        let num = typeof value === "string" ? BigInt(value) : BigInt(value || 0);
        const d = BigInt("1000000000000000000");
        return Math.floor(Number(num) / Number(d)).toLocaleString("en-US");
      } catch { return "0"; }
    },
    truncateHash(hash) {
      if (!hash) return "";
      return hash.slice(0, 8) + "…" + hash.slice(-6);
    },
    truncateAddr(addr) {
      if (!addr) return "";
      if (addr === "Block_Reward") return addr;
      return addr.slice(0, 8) + "…" + addr.slice(-4);
    },
    timeAgo(ts) {
      const diff = Math.floor((Date.now() - new Date(ts).getTime()) / 1000);
      if (diff < 60) return diff + "s ago";
      if (diff < 3600) return Math.floor(diff / 60) + "m ago";
      if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
      return Math.floor(diff / 86400) + "d ago";
    },
  },
};
</script>

<style scoped>
/* ── Hero ── */
.hero {
  position: relative;
  text-align: center;
  padding: 5rem 1rem 4rem;
  margin: -2rem -2rem 2rem;
  overflow: hidden;
}

.hero-bg {
  position: absolute; inset: 0;
  background:
    radial-gradient(ellipse 80% 60% at 50% -10%, rgba(59,130,246,.16) 0%, transparent 70%),
    radial-gradient(ellipse 50% 40% at 80% 80%, rgba(139,92,246,.1) 0%, transparent 60%);
}

.hero-grid {
  position: absolute; inset: 0;
  background-image:
    linear-gradient(rgba(59,130,246,.04) 1px, transparent 1px),
    linear-gradient(90deg, rgba(59,130,246,.04) 1px, transparent 1px);
  background-size: 50px 50px;
}

.hero-content {
  position: relative;
  max-width: 680px;
  margin: 0 auto;
}

.hero-badge {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  background: rgba(59,130,246,.1);
  border: 1px solid rgba(59,130,246,.25);
  border-radius: 100px;
  padding: 5px 16px;
  font-size: 12px;
  color: var(--accent);
  margin-bottom: 1.5rem;
}

.badge-dot {
  width: 6px; height: 6px;
  background: var(--accent);
  border-radius: 50%;
  box-shadow: 0 0 6px var(--accent);
  animation: pulse-dot 2s infinite;
}

@keyframes pulse-dot {
  0%,100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.hero h1 {
  font-size: 3rem;
  font-weight: 800;
  color: var(--text);
  line-height: 1.15;
  margin-bottom: 0.75rem;
}

.hero-accent { color: var(--accent); }

.hero-sub {
  color: var(--text2);
  font-size: 1rem;
  margin-bottom: 2rem;
}

.search-box {
  display: flex;
  gap: 0;
  max-width: 580px;
  margin: 0 auto;
  border: 1px solid var(--border2);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--surface);
}

.search-box input {
  flex: 1;
  padding: 0.75rem 1.25rem;
  background: transparent;
  border: none;
  color: var(--text);
  font-size: 14px;
  outline: none;
}

.search-box input::placeholder { color: var(--muted); }

.search-box button {
  padding: 0.75rem 1.5rem;
  background: var(--accent);
  color: #fff;
  border: none;
  cursor: pointer;
  font-weight: 600;
  font-size: 14px;
  transition: background 0.2s;
}

.search-box button:hover { background: #2563eb; }

/* ── Stats ── */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 1px;
  background: var(--border);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  margin-bottom: 2rem;
}

.stat-card {
  background: var(--surface);
  padding: 1.5rem 1.25rem;
  text-align: center;
  transition: background 0.2s;
}

.stat-card:hover { background: var(--surface2); }

.accent-card { background: rgba(59,130,246,.07); }
.accent-card:hover { background: rgba(59,130,246,.12); }
.green-card { background: rgba(16,185,129,.06); }
.green-card:hover { background: rgba(16,185,129,.1); }

.stat-label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  color: var(--text2);
  margin-bottom: 0.4rem;
}

.stat-value {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--text);
  line-height: 1.2;
  word-break: break-word;
}

.accent-card .stat-value { color: var(--accent); }
.green-card .stat-value { color: var(--green); }

.stat-sub {
  font-size: 11px;
  color: var(--muted);
  margin-top: 0.25rem;
}

.stats-placeholder {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 1px;
  background: var(--border);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  margin-bottom: 2rem;
}

.stat-skeleton {
  background: var(--surface);
  height: 90px;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0%,100% { opacity: 0.6; }
  50% { opacity: 1; }
}

/* ── Recent Grid ── */
.recent-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5rem;
}

.panel {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1rem 1.5rem;
  border-bottom: 1px solid var(--border);
}

.panel-header h2 {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.panel-link {
  font-size: 12px;
  color: var(--text2);
  transition: color 0.2s;
}

.panel-link:hover { color: var(--accent); }

.list { display: flex; flex-direction: column; }

.list-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.875rem 1.5rem;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.15s;
}

.list-row:last-child { border-bottom: none; }
.list-row:hover { background: var(--surface2); }

.row-icon {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  flex-shrink: 0;
}

.block-icon { background: rgba(59,130,246,.12); }
.mining-icon { background: rgba(245,158,11,.12); }
.tx-icon { background: rgba(139,92,246,.12); }

.row-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.row-title {
  font-size: 13px;
  font-weight: 600;
  font-family: var(--mono);
}

.row-sub {
  font-size: 11px;
  color: var(--text2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.row-right {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
}

.row-badge {
  font-size: 11px;
  background: rgba(59,130,246,.12);
  color: var(--accent);
  border-radius: 4px;
  padding: 1px 6px;
}

.row-amount {
  font-size: 12px;
  font-weight: 600;
}

.row-time {
  font-size: 11px;
  color: var(--muted);
}

.badge-mining {
  background: rgba(245,158,11,.15);
  color: var(--yellow);
  border-radius: 4px;
  padding: 1px 6px;
  font-size: 11px;
}

.accent-text { color: var(--accent); }
.green-text { color: var(--green); }

.panel-empty {
  padding: 3rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin {
  display: inline-block;
  animation: spin 1.5s linear infinite;
  margin-right: 6px;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to   { transform: rotate(360deg); }
}

@media (max-width: 1200px) {
  .stats-grid, .stats-placeholder {
    grid-template-columns: repeat(3, 1fr);
  }
}

@media (max-width: 768px) {
  .hero { padding: 3rem 1rem 2.5rem; margin: -1rem -1rem 1.5rem; }
  .hero h1 { font-size: 2rem; }
  .stats-grid, .stats-placeholder { grid-template-columns: repeat(2, 1fr); }
  .recent-grid { grid-template-columns: 1fr; }
}

@media (max-width: 480px) {
  .stats-grid, .stats-placeholder { grid-template-columns: 1fr 1fr; }
}
</style>
