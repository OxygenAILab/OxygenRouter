import React from "react";
import { Link } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { PageHeader } from "../App";
import Button from "../components/ui/Button";
import Input from "../components/ui/Input";
import { api } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useToast } from "../lib/toast";

const money = (micros: number) => new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(micros / 1_000_000);

export default function WalletPage() {
  const { user } = useAuth(); const toast = useToast(); const qc = useQueryClient();
  const { data, isLoading, error } = useQuery({ queryKey: ["wallet"], queryFn: api.wallet.get, enabled: !!user });
  const [amount, setAmount] = React.useState(""); const [code, setCode] = React.useState("");
  const submitOrder = async (e: React.FormEvent) => { e.preventDefault(); const micros = Math.round(Number(amount) * 1_000_000); if (!Number.isFinite(micros) || micros <= 0) return toast.error("Enter a valid amount"); try { await api.orders.manual(micros); toast.success("Manual order requested", "An administrator will review it."); setAmount(""); } catch (err) { toast.error("Could not create order", err instanceof Error ? err.message : undefined); } };
  const redeem = async (e: React.FormEvent) => { e.preventDefault(); try { await api.redemption.redeem(code); toast.success("Code redeemed"); setCode(""); qc.invalidateQueries({ queryKey: ["wallet"] }); } catch (err) { toast.error("Could not redeem code", err instanceof Error ? err.message : undefined); } };
  if (!user) return <AccessNote />;
  return <><PageHeader title="Wallet" description="Balance, credits, and payment requests." action={<Link className="btn-outlined" to="/ui/plans">Browse plans</Link>} /><section className="wallet-balance"><span>Available credit</span><strong>{data ? money(data.balance_micros) : "..."}</strong><small>Stored in USD micros</small></section><div className="billing-grid"><section className="dashboard-section billing-form"><div className="section-heading"><h2>Add funds</h2></div><form onSubmit={submitOrder}><Input label="Amount (USD)" inputMode="decimal" placeholder="25.00" value={amount} onChange={(e) => setAmount(e.target.value)} /><Button type="submit">Request manual payment</Button></form></section><section className="dashboard-section billing-form"><div className="section-heading"><h2>Redeem code</h2></div><form onSubmit={redeem}><Input label="Redemption code" value={code} onChange={(e) => setCode(e.target.value)} /><Button type="submit" variant="outlined">Redeem</Button></form></section></div><section className="dashboard-section ledger-section"><div className="section-heading"><h2>Ledger</h2></div>{isLoading ? <div className="empty-state">Loading ledger...</div> : error ? <div className="error-banner">{(error as Error).message}</div> : data?.ledger.length ? <table className="table"><thead><tr><th>When</th><th>Type</th><th>Description</th><th>Amount</th><th>Balance</th></tr></thead><tbody>{data.ledger.map((row) => <tr key={row.id}><td>{new Date(row.created_at).toLocaleDateString()}</td><td>{row.kind}</td><td>{row.description}</td><td>{money(row.amount_micros)}</td><td>{money(row.balance_after_micros)}</td></tr>)}</tbody></table> : <div className="empty-state">No balance activity yet.</div>}</section></>;
}

function AccessNote() { return <div className="empty-state">Sign in to access your wallet.</div>; }
