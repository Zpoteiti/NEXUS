import React from 'react'

export const inputStyle: React.CSSProperties = {
  background: 'rgba(255,255,255,0.05)',
  border: '1px solid rgba(255,255,255,0.08)',
}

export const cardStyle: React.CSSProperties = {
  background: '#0f172a',
  border: '1px solid rgba(255,255,255,0.08)',
}

export function SaveButton({ onClick, saved, label = 'Save' }: { onClick: () => void; saved: boolean; label?: string }) {
  return (
    <div className="flex items-center gap-3">
      <button
        onClick={onClick}
        className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
        style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 20px rgba(99, 102, 241, 0.2)' }}
      >
        {label}
      </button>
      {saved && <span className="text-sm" style={{ color: '#22c55e' }}>Saved!</span>}
    </div>
  )
}
