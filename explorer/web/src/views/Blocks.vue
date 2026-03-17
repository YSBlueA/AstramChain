<template>
  <div class="page">
    <div class="page-header">
      <h1>Blocks</h1>
      <span class="page-count" v-if="total">{{ total.toLocaleString() }} total</span>
    </div>

    <div class="card">
      <div v-if="blocks.length" class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Height</th>
              <th>Hash</th>
              <th>Miner</th>
              <th class="center">Txs</th>
              <th>Timestamp</th>
              <th class="center">Difficulty</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="block in blocks"
              :key="block.hash"
              @click="$router.push(`/blocks/${block.height}`)"
              class="row-link"
            >
              <td><span class="accent-text bold">#{{ block.height }}</span></td>
              <td><span class="mono muted">{{ truncateHash(block.hash) }}</span></td>
              <td><span class="mono muted">{{ truncateAddr(block.miner) }}</span></td>
              <td class="center"><span class="badge-count">{{ block.transactions }}</span></td>
              <td class="muted">{{ formatTime(block.timestamp) }}</td>
              <td class="center"><span class="diff-badge">{{ block.difficulty }}</span></td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="empty">
        <span class="spin">⟳</span> Loading blocks...
      </div>

      <div class="pagination">
        <button @click="prevPage" :disabled="currentPage === 1" class="page-btn">← Prev</button>
        <span class="page-info">Page {{ currentPage }} of {{ totalPages }}</span>
        <button @click="nextPage" :disabled="currentPage >= totalPages" class="page-btn">Next →</button>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";
export default {
  name: "Blocks",
  data() {
    return { blocks: [], currentPage: 1, limit: 20, total: 0, interval: null };
  },
  computed: {
    totalPages() { return Math.max(1, Math.ceil(this.total / this.limit)); },
  },
  mounted() {
    this.fetch();
    this.interval = setInterval(() => { if (this.currentPage === 1) this.fetch(); }, 5000);
  },
  beforeUnmount() { clearInterval(this.interval); },
  methods: {
    async fetch() {
      try {
        const res = await explorerAPI.getBlocks(this.currentPage, this.limit);
        this.blocks = res.data.blocks;
        this.total = res.data.total;
      } catch (e) { console.error(e); }
    },
    prevPage() { if (this.currentPage > 1) { this.currentPage--; this.fetch(); } },
    nextPage() { if (this.currentPage < this.totalPages) { this.currentPage++; this.fetch(); } },
    formatTime(ts) { return new Date(ts).toLocaleString("ko-KR"); },
    truncateHash(h) { return h ? h.slice(0, 10) + "…" + h.slice(-6) : ""; },
    truncateAddr(a) { return a ? a.slice(0, 8) + "…" + a.slice(-4) : ""; },
  },
};
</script>

<style scoped>
.page { width: 100%; }

.page-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

h1 { font-size: 1.5rem; font-weight: 700; color: var(--text); }

.page-count {
  font-size: 12px;
  color: var(--text2);
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 100px;
  padding: 3px 10px;
}

.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.table-wrap { overflow-x: auto; }

table {
  width: 100%;
  border-collapse: collapse;
}

thead tr {
  border-bottom: 1px solid var(--border);
}

th {
  padding: 0.75rem 1.25rem;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
  text-align: left;
  white-space: nowrap;
}

td {
  padding: 0.875rem 1.25rem;
  font-size: 13px;
  border-bottom: 1px solid var(--border);
  color: var(--text);
}

tbody tr:last-child td { border-bottom: none; }

.row-link { cursor: pointer; transition: background 0.15s; }
.row-link:hover td { background: var(--surface2); }

.accent-text { color: var(--accent); }
.bold { font-weight: 700; }
.mono { font-family: var(--mono); font-size: 12px; }
.muted { color: var(--text2); }
.center { text-align: center; }

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

.pagination {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 1rem;
  padding: 1rem;
  border-top: 1px solid var(--border);
}

.page-btn {
  padding: 0.4rem 1rem;
  background: var(--surface2);
  border: 1px solid var(--border2);
  border-radius: var(--radius);
  color: var(--text2);
  cursor: pointer;
  font-size: 13px;
  transition: all 0.2s;
}

.page-btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}

.page-btn:disabled { opacity: 0.35; cursor: not-allowed; }

.page-info { color: var(--text2); font-size: 13px; }

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
</style>
