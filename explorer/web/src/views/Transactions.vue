<template>
  <div class="page">
    <div class="page-header">
      <h1>Transactions</h1>
      <span class="page-count" v-if="total">{{ total.toLocaleString() }} total</span>
    </div>

    <div class="card">
      <div v-if="transactions.length" class="table-wrap">
        <table>
          <thead>
            <tr>
              <th class="center">Type</th>
              <th>Hash</th>
              <th>From</th>
              <th>To</th>
              <th class="right">Amount</th>
              <th class="right">Fee</th>
              <th class="center">Status</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="tx in transactions"
              :key="tx.hash"
              @click="$router.push(`/transactions/${tx.hash}`)"
              class="row-link"
            >
              <td class="center">
                <span v-if="tx.from === 'Block_Reward'" class="type-badge mining" title="Mining Reward">⛏</span>
                <span v-else class="type-badge transfer" title="Transfer">⇄</span>
              </td>
              <td><span class="mono accent-text">{{ truncateHash(tx.hash) }}</span></td>
              <td><span class="mono text2">{{ truncateAddr(tx.from) }}</span></td>
              <td><span class="mono text2">{{ truncateAddr(tx.to) }}</span></td>
              <td class="right green-text bold">{{ formatAmount(tx.amount) }}</td>
              <td class="right yellow-text">{{ formatAmount(tx.fee) }}</td>
              <td class="center">
                <span class="status-badge" :class="tx.status">{{ tx.status }}</span>
              </td>
              <td class="text2 small">{{ formatTime(tx.timestamp) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="empty"><span class="spin">⟳</span> Loading transactions...</div>

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
  name: "Transactions",
  data() { return { transactions: [], currentPage: 1, limit: 20, total: 0 }; },
  computed: {
    totalPages() { return Math.max(1, Math.ceil(this.total / this.limit)); },
  },
  mounted() { this.fetch(); },
  methods: {
    async fetch() {
      try {
        const res = await explorerAPI.getTransactions(this.currentPage, this.limit);
        this.transactions = res.data.transactions;
        this.total = res.data.total;
      } catch (e) { console.error(e); }
    },
    prevPage() { if (this.currentPage > 1) { this.currentPage--; this.fetch(); } },
    nextPage() { if (this.currentPage < this.totalPages) { this.currentPage++; this.fetch(); } },
    formatTime(ts) { return new Date(ts).toLocaleString("ko-KR"); },
    formatAmount(value) {
      try {
        const n = BigInt(value || 0);
        const d = BigInt("1000000000000000000");
        return (Number(n) / Number(d)).toLocaleString("en-US", { minimumFractionDigits: 0, maximumFractionDigits: 4 });
      } catch { return "0"; }
    },
    truncateHash(h) { return h ? h.slice(0, 10) + "…" + h.slice(-6) : ""; },
    truncateAddr(a) {
      if (!a) return "";
      if (a === "Block_Reward") return "Block Reward";
      return a.slice(0, 8) + "…" + a.slice(-4);
    },
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

table { width: 100%; border-collapse: collapse; }

thead tr { border-bottom: 1px solid var(--border); }

th {
  padding: 0.75rem 1rem;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
  text-align: left;
  white-space: nowrap;
}

td {
  padding: 0.75rem 1rem;
  font-size: 13px;
  border-bottom: 1px solid var(--border);
  color: var(--text);
  white-space: nowrap;
}

tbody tr:last-child td { border-bottom: none; }
.row-link { cursor: pointer; transition: background 0.15s; }
.row-link:hover td { background: var(--surface2); }

.type-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px; height: 28px;
  border-radius: 8px;
  font-size: 13px;
}

.type-badge.mining { background: rgba(245,158,11,.12); }
.type-badge.transfer { background: rgba(139,92,246,.12); }

.mono { font-family: var(--mono); font-size: 12px; }
.accent-text { color: var(--accent); }
.text2 { color: var(--text2); }
.green-text { color: var(--green); }
.yellow-text { color: var(--yellow); }
.bold { font-weight: 600; }
.small { font-size: 12px; }
.center { text-align: center; }
.right { text-align: right; }

.status-badge {
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 600;
  text-transform: capitalize;
}
.status-badge.confirmed { background: rgba(16,185,129,.12); color: var(--green); }
.status-badge.pending { background: rgba(245,158,11,.12); color: var(--yellow); }

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
.page-btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
.page-btn:disabled { opacity: 0.35; cursor: not-allowed; }
.page-info { color: var(--text2); font-size: 13px; }

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin { display: inline-block; animation: spin 1.5s linear infinite; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }
</style>
