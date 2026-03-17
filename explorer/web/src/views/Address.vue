<template>
  <div class="page">
    <div v-if="addressInfo">
      <div class="page-header">
        <button class="back-btn" @click="$router.push('/')">← Home</button>
        <h1>Address</h1>
      </div>

      <!-- Address hash -->
      <div class="addr-card">
        <div class="addr-label">Wallet Address</div>
        <div class="addr-hash mono">{{ addressInfo.address }}</div>
      </div>

      <!-- Stats row -->
      <div class="stats-row">
        <div class="stat-box highlight">
          <div class="stat-label">Balance</div>
          <div class="stat-val accent-text">{{ formatAmount(addressInfo.balance) }} <span class="unit">ASRM</span></div>
        </div>
        <div class="stat-box">
          <div class="stat-label">Total Received</div>
          <div class="stat-val green-text">{{ formatAmount(addressInfo.received) }} <span class="unit">ASRM</span></div>
        </div>
        <div class="stat-box">
          <div class="stat-label">Total Sent</div>
          <div class="stat-val yellow-text">{{ formatAmount(addressInfo.sent) }} <span class="unit">ASRM</span></div>
        </div>
        <div class="stat-box">
          <div class="stat-label">Transactions</div>
          <div class="stat-val">{{ addressInfo.transaction_count }}</div>
        </div>
        <div class="stat-box">
          <div class="stat-label">Last Activity</div>
          <div class="stat-val small">{{ addressInfo.last_transaction ? formatTime(addressInfo.last_transaction) : 'No activity' }}</div>
        </div>
      </div>
    </div>

    <div v-else class="empty"><span class="spin">⟳</span> Loading address...</div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";
export default {
  name: "Address",
  data() { return { addressInfo: null }; },
  mounted() { this.fetch(); },
  methods: {
    async fetch() {
      try {
        const res = await explorerAPI.getAddressInfo(this.$route.params.address);
        this.addressInfo = res.data;
      } catch (e) { console.error(e); }
    },
    formatTime(ts) { return new Date(ts).toLocaleString("ko-KR"); },
    formatAmount(value) {
      try {
        let n;
        if (Array.isArray(value)) n = BigInt(value[0]) + (BigInt(value[1]) << 64n) + (BigInt(value[2]) << 128n) + (BigInt(value[3]) << 192n);
        else n = BigInt(value || 0);
        return (Number(n) / 1e18).toLocaleString("en-US", { minimumFractionDigits: 0, maximumFractionDigits: 6 });
      } catch { return "0"; }
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

.addr-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--radius-lg);
  padding: 1.25rem 1.5rem;
  margin-bottom: 1.25rem;
}

.addr-label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
  margin-bottom: 0.5rem;
}

.addr-hash {
  font-size: 13px;
  color: var(--text);
  word-break: break-all;
}

.stats-row {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 1px;
  background: var(--border);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.stat-box {
  background: var(--surface);
  padding: 1.25rem 1.5rem;
  transition: background 0.15s;
}
.stat-box:hover { background: var(--surface2); }
.stat-box.highlight { background: rgba(59,130,246,.07); }
.stat-box.highlight:hover { background: rgba(59,130,246,.12); }

.stat-label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
  margin-bottom: 0.4rem;
}

.stat-val {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text);
}

.stat-val.small { font-size: 0.85rem; }

.unit { font-size: 0.7rem; font-weight: 400; color: var(--muted); }
.mono { font-family: var(--mono); }
.accent-text { color: var(--accent); }
.green-text { color: var(--green); }
.yellow-text { color: var(--yellow); }

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin { display: inline-block; animation: spin 1.5s linear infinite; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }
</style>
