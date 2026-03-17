<template>
  <div class="page">
    <div v-if="loading" class="empty"><span class="spin">⟳</span> Loading transaction...</div>

    <div v-else-if="error" class="error-card">
      <div class="error-icon">✕</div>
      <h2>Transaction not found</h2>
      <p>{{ error }}</p>
      <code class="error-hash">{{ searchHash }}</code>
      <button class="back-btn" @click="$router.push('/transactions')">← All Transactions</button>
    </div>

    <div v-else-if="transaction">
      <div class="page-header">
        <button class="back-btn" @click="$router.push('/transactions')">← Transactions</button>
        <h1>Transaction</h1>
        <span v-if="isCoinbase" class="type-tag mining">⛏ Mining Reward</span>
        <span v-else class="type-tag transfer">⇄ Transfer</span>
        <span class="status-badge" :class="transaction.status">{{ transaction.status }}</span>
      </div>

      <div class="detail-grid">
        <div class="detail-row">
          <span class="detail-label">Hash</span>
          <span class="detail-value mono">{{ transaction.hash }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">From</span>
          <span
            class="detail-value mono"
            :class="isCoinbase ? 'yellow-text' : 'accent-text clickable'"
            @click="!isCoinbase && $router.push(`/address/${transaction.from}`)"
          >{{ transaction.from }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">To</span>
          <span
            class="detail-value mono"
            :class="isClickableAddr(transaction.to) ? 'accent-text clickable' : 'text2'"
            @click="isClickableAddr(transaction.to) && $router.push(`/address/${transaction.to}`)"
          >{{ transaction.to }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">{{ isCoinbase ? 'Reward Amount' : 'Amount' }}</span>
          <span class="detail-value green-text bold">{{ formatAmount(transaction.amount) }} ASRM</span>
        </div>
        <div v-if="!isCoinbase" class="detail-row">
          <span class="detail-label">Fee</span>
          <span class="detail-value yellow-text">
            {{ formatAmount(transaction.fee) }} ASRM
            <span class="sub-text">({{ formatRam(transaction.fee) }} ram)</span>
          </span>
        </div>
        <div v-if="!isCoinbase" class="detail-row">
          <span class="detail-label">Total</span>
          <span class="detail-value accent-text bold">{{ formatTotal(transaction.amount, transaction.fee) }} ASRM</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Block</span>
          <span
            class="detail-value accent-text clickable bold"
            @click="transaction.block_height && $router.push(`/blocks/${transaction.block_height}`)"
          >{{ transaction.block_height ? '#' + transaction.block_height : 'Pending' }}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Confirmations</span>
          <span class="detail-value" :class="getConfirmClass(transaction.confirmations)">
            {{ getConfirmText(transaction.confirmations) }}
          </span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Timestamp</span>
          <span class="detail-value text2">{{ formatTime(transaction.timestamp) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";
export default {
  name: "TransactionDetail",
  data() { return { transaction: null, loading: false, error: null, searchHash: "" }; },
  computed: {
    isCoinbase() { return this.transaction?.from === "Block_Reward"; },
  },
  mounted() { this.fetch(); },
  methods: {
    async fetch() {
      this.loading = true; this.error = null;
      try {
        const hash = this.$route.params.hash;
        this.searchHash = hash;
        const res = await explorerAPI.getTransactionByHash(hash);
        this.transaction = res.data;
      } catch (e) {
        this.error = e.response?.data?.error || "Transaction not found.";
      } finally { this.loading = false; }
    },
    isClickableAddr(addr) { return addr && !addr.includes("recipients") && !addr.includes("outputs"); },
    formatTime(ts) { return new Date(ts).toLocaleString("ko-KR"); },
    formatAmount(value) {
      try {
        let n;
        if (Array.isArray(value)) n = BigInt(value[0]) + (BigInt(value[1]) << 64n) + (BigInt(value[2]) << 128n) + (BigInt(value[3]) << 192n);
        else n = BigInt(value || 0);
        return (Number(n) / 1e18).toLocaleString("en-US", { minimumFractionDigits: 0, maximumFractionDigits: 6 });
      } catch { return "0"; }
    },
    formatTotal(amount, fee) {
      try {
        const a = BigInt(amount || 0);
        const f = BigInt(fee || 0);
        return (Number(a + f) / 1e18).toLocaleString("en-US", { minimumFractionDigits: 0, maximumFractionDigits: 6 });
      } catch { return "0"; }
    },
    formatRam(value) {
      try { return BigInt(value || 0).toLocaleString("en-US"); } catch { return "0"; }
    },
    getConfirmClass(c) {
      if (c == null) return "text2";
      return c >= 6 ? "green-text" : c > 0 ? "yellow-text" : "red-text";
    },
    getConfirmText(c) {
      if (c == null) return "Pending";
      if (c === 0) return "0 — Unconfirmed";
      if (c < 6) return `${c} — Low Confidence`;
      return `${c} — Confirmed`;
    },
  },
};
</script>

<style scoped>
.page { width: 100%; }

.page-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex-wrap: wrap;
  margin-bottom: 1.5rem;
}

h1 { font-size: 1.5rem; font-weight: 700; color: var(--text); }

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

.type-tag {
  padding: 3px 10px;
  border-radius: 100px;
  font-size: 12px;
  font-weight: 600;
}
.type-tag.mining { background: rgba(245,158,11,.12); color: var(--yellow); }
.type-tag.transfer { background: rgba(139,92,246,.12); color: #a78bfa; }

.status-badge {
  padding: 3px 10px;
  border-radius: 100px;
  font-size: 12px;
  font-weight: 600;
  text-transform: capitalize;
}
.status-badge.confirmed { background: rgba(16,185,129,.12); color: var(--green); }
.status-badge.pending { background: rgba(245,158,11,.12); color: var(--yellow); }

.detail-grid {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.detail-row {
  display: grid;
  grid-template-columns: 180px 1fr;
  padding: 0.875rem 1.5rem;
  border-bottom: 1px solid var(--border);
  align-items: start;
  gap: 1rem;
}
.detail-row:last-child { border-bottom: none; }

.detail-label {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text2);
  padding-top: 2px;
}

.detail-value {
  font-size: 13px;
  color: var(--text);
  word-break: break-all;
}

.sub-text { font-size: 11px; color: var(--muted); display: block; margin-top: 2px; }

.mono { font-family: var(--mono); font-size: 12px; }
.text2 { color: var(--text2); }
.accent-text { color: var(--accent); }
.green-text { color: var(--green); }
.yellow-text { color: var(--yellow); }
.red-text { color: var(--red); }
.bold { font-weight: 600; }
.clickable { cursor: pointer; }
.clickable:hover { text-decoration: underline; }

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
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  font-weight: 700;
}

.error-card h2 { color: var(--red); font-size: 1.2rem; }
.error-card p { color: var(--text2); font-size: 13px; }

.error-hash {
  font-family: var(--mono);
  font-size: 11px;
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.5rem 1rem;
  color: var(--text2);
  word-break: break-all;
  max-width: 100%;
}

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin { display: inline-block; animation: spin 1.5s linear infinite; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 640px) {
  .detail-row { grid-template-columns: 1fr; gap: 0.25rem; }
}
</style>
