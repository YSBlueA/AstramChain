<template>
  <div class="page">
    <div v-if="block">
      <div class="page-header">
        <button class="back-btn" @click="$router.push('/blocks')">← Blocks</button>
        <h1>Block <span class="accent-text">#{{ block.height }}</span></h1>
        <span
          class="confirm-badge"
          :class="block.confirmations >= 6 ? 'confirmed' : block.confirmations > 0 ? 'low' : 'unconfirmed'"
        >
          {{ block.confirmations >= 6 ? '✔ Confirmed' : block.confirmations > 0 ? '⏳ Low Confidence' : '⚠ Unconfirmed' }}
        </span>
      </div>

      <div class="detail-grid">
        <div class="detail-row">
          <span class="detail-label">Block Hash</span>
          <span class="detail-value mono">{{ block.hash }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Previous Hash</span>
          <span class="detail-value mono text2">{{ block.previous_hash }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Miner</span>
          <span
            class="detail-value mono accent-text clickable"
            @click="$router.push(`/address/${block.miner}`)"
          >{{ block.miner }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Timestamp</span>
          <span class="detail-value">{{ formatTime(block.timestamp) }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Transactions</span>
          <span class="detail-value"><span class="badge-count">{{ block.transactions }}</span></span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Difficulty</span>
          <span class="detail-value"><span class="diff-badge">{{ block.difficulty }}</span></span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Nonce</span>
          <span class="detail-value mono text2">{{ block.nonce }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Confirmations</span>
          <span class="detail-value">{{ block.confirmations }}</span>
        </div>
      </div>
    </div>
    <div v-else class="empty"><span class="spin">⟳</span> Loading block...</div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";
export default {
  name: "BlockDetail",
  data() { return { block: null }; },
  mounted() { this.fetch(); },
  methods: {
    async fetch() {
      try {
        const h = this.$route.params.height;
        const res = /^[A-Fa-f0-9]{60,}$/.test(h)
          ? await explorerAPI.getBlockByHash(h)
          : await explorerAPI.getBlockByHeight(h);
        this.block = res.data;
      } catch (e) { console.error(e); }
    },
    formatTime(ts) { return new Date(ts).toLocaleString("ko-KR"); },
  },
};
</script>

<style scoped>
.page { width: 100%; }

.page-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  flex-wrap: wrap;
  margin-bottom: 1.5rem;
}

h1 { font-size: 1.5rem; font-weight: 700; color: var(--text); }
.accent-text { color: var(--accent); }

.back-btn {
  padding: 0.35rem 0.9rem;
  background: var(--surface2);
  border: 1px solid var(--border2);
  border-radius: var(--radius);
  color: var(--text2);
  cursor: pointer;
  font-size: 13px;
  transition: all 0.2s;
}
.back-btn:hover { border-color: var(--accent); color: var(--accent); }

.confirm-badge {
  padding: 4px 12px;
  border-radius: 100px;
  font-size: 12px;
  font-weight: 600;
}
.confirm-badge.confirmed { background: rgba(16,185,129,.12); color: var(--green); }
.confirm-badge.low { background: rgba(245,158,11,.12); color: var(--yellow); }
.confirm-badge.unconfirmed { background: rgba(239,68,68,.12); color: var(--red); }

.detail-grid {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.detail-row {
  display: grid;
  grid-template-columns: 200px 1fr;
  padding: 0.875rem 1.5rem;
  border-bottom: 1px solid var(--border);
  align-items: start;
  gap: 1rem;
}
.detail-row:last-child { border-bottom: none; }

.detail-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding-top: 2px;
}

.detail-value {
  font-size: 13px;
  color: var(--text);
  word-break: break-all;
}

.mono { font-family: var(--mono); font-size: 12px; }
.text2 { color: var(--text2); }
.clickable { cursor: pointer; }
.clickable:hover { text-decoration: underline; }

.badge-count {
  background: rgba(59,130,246,.1);
  color: var(--accent);
  border-radius: 6px;
  padding: 2px 8px;
  font-size: 12px;
}

.diff-badge {
  background: rgba(139,92,246,.1);
  color: #a78bfa;
  border-radius: 6px;
  padding: 2px 8px;
  font-size: 12px;
  font-weight: 600;
}

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin {
  display: inline-block;
  animation: spin 1.5s linear infinite;
  margin-right: 6px;
}
@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 640px) {
  .detail-row { grid-template-columns: 1fr; gap: 0.25rem; }
}
</style>
