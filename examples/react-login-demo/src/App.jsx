import React, { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import './App.css';

const API_BASE = '/api';

/**
 * UTILS & HOOKS
 */

const useCountUp = (target, duration = 1500) => {
  const [count, setCount] = useState(0);
  const countRef = useRef(0);
  const startTime = useRef(null);

  useEffect(() => {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      setCount(target);
      return;
    }

    const animate = (timestamp) => {
      if (!startTime.current) startTime.current = timestamp;
      const progress = Math.min((timestamp - startTime.current) / duration, 1);
      
      // Ease out quad
      const eased = progress === 1 ? 1 : 1 - Math.pow(2, -10 * progress);
      const nextCount = eased * target;
      
      setCount(nextCount);
      
      if (progress < 1) {
        requestAnimationFrame(animate);
      }
    };

    startTime.current = null;
    requestAnimationFrame(animate);
  }, [target, duration]);

  return count;
};

const formatCurrency = (val) => new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2
}).format(val);

const getInitials = (name) => {
  if (!name) return '??';
  return name.split(' ').map(n => n[0]).join('').toUpperCase().slice(0, 2);
};

const stringToColor = (str) => {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = str.charCodeAt(i) + ((hash << 5) - hash);
  }
  return `hsl(${hash % 360}, 60%, 50%)`;
};

/**
 * ICONS (Inline SVG)
 */

const Icon = ({ name, size = 20, className = '' }) => {
  const icons = {
    search: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>
      </svg>
    ),
    logout: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" x2="9" y1="12" y2="12"/>
      </svg>
    ),
    dollar: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="pulse-icon">
        <line x1="12" x2="12" y1="2" y2="22"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/>
      </svg>
    ),
    gear: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ animation: 'spin 4s linear infinite' }}>
        <path d="M12.22 2h-.44a2 2 0 0 0-2 2 2 2 0 0 1-2 2 2 2 0 0 0-2 2 2 2 0 0 1-2 2 2 2 0 0 0-2 2v.44a2 2 0 0 0 2 2 2 2 0 0 1 2 2 2 2 0 0 0 2 2 2 2 0 0 1 2 2 2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2 2 2 0 0 1 2-2 2 2 0 0 0 2-2 2 2 0 0 1 2-2 2 2 0 0 0 2-2v-.44a2 2 0 0 0-2-2 2 2 0 0 1-2-2 2 2 0 0 0-2-2 2 2 0 0 1-2-2 2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>
      </svg>
    ),
    chart: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <line x1="18" y1="20" x2="18" y2="10" className="bar-anim-1" style={{ transformOrigin: 'bottom', animation: 'growY 1s ease-out forwards' }}/><line x1="12" y1="20" x2="12" y2="4" className="bar-anim-2" style={{ transformOrigin: 'bottom', animation: 'growY 1s ease-out 0.2s forwards' }}/><line x1="6" y1="20" x2="6" y2="14" className="bar-anim-3" style={{ transformOrigin: 'bottom', animation: 'growY 1s ease-out 0.4s forwards' }}/>
      </svg>
    ),
    layers: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/>
      </svg>
    ),
    spinner: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" className="spin-icon" style={{ animation: 'spin 1s linear infinite' }}>
        <path d="M21 12a9 9 0 1 1-6.219-8.56" />
      </svg>
    ),
    alert: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
    ),
    empty: (
      <svg width={48} height={48} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: 0.3 }}>
        <circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/><line x1="8" y1="11" x2="14" y2="11"/>
      </svg>
    ),
    logo: (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <linearGradient id="logoGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#6366F1" />
            <stop offset="100%" stopColor="#10B981" />
          </linearGradient>
        </defs>
        <rect x="2" y="2" width="20" height="20" rx="6" fill="url(#logoGrad)" />
        <path d="M7 12L10 15L17 8" stroke="white" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    )
  };
  return <span className={className}>{icons[name] || null}</span>;
};

/**
 * COMPONENTS
 */

const StatCard = ({ title, value, caption, icon, colorClass, delay = 0, isCurrency = false }) => {
  const animatedValue = useCountUp(value);
  
  return (
    <div className="stat-card" style={{ animationDelay: `${delay}ms` }}>
      <div className="stat-header">
        <div className={`stat-icon-box ${colorClass}`}>
          <Icon name={icon} />
        </div>
        <span className="stat-title">{title}</span>
      </div>
      <div className="stat-value">
        {isCurrency ? formatCurrency(animatedValue) : Math.round(animatedValue).toLocaleString()}
      </div>
      {caption && <div className="stat-caption" title={caption}>{caption}</div>}
    </div>
  );
};

const PriceBarChart = ({ data }) => {
  const maxPrice = useMemo(() => {
    return Math.max(...data.flatMap(p => [p.price_input_per_1m, p.price_output_per_1m]), 1);
  }, [data]);

  const [tooltip, setTooltip] = useState(null);

  const handleMouseEnter = (provider, e) => {
    const rect = e.currentTarget.getBoundingClientRect();
    setTooltip({
      name: provider.name,
      input: provider.price_input_per_1m,
      output: provider.price_output_per_1m,
      x: rect.left + rect.width / 2,
      y: rect.top - 10
    });
  };

  return (
    <div className="panel-card">
      <div className="panel-header">
        <h3 className="panel-title">Pricing Comparison</h3>
        <p className="panel-subtitle">Input vs Output rates per 1M tokens</p>
      </div>
      
      <div className="bar-chart-container">
        {data.map((p, idx) => (
          <div 
            key={p.name} 
            className="bar-row" 
            onMouseEnter={(e) => handleMouseEnter(p, e)}
            onMouseLeave={() => setTooltip(null)}
          >
            <div className="row-label">{p.name}</div>
            <div className="bar-group">
              <div className="bar-track">
                <div 
                  className="bar-fill input" 
                  style={{ width: `${(p.price_input_per_1m / maxPrice) * 100}%`, animationDelay: `${idx * 100}ms` }}
                />
                <span className="bar-value">${p.price_input_per_1m.toFixed(2)}</span>
              </div>
              <div className="bar-track">
                <div 
                  className="bar-fill output" 
                  style={{ width: `${(p.price_output_per_1m / maxPrice) * 100}%`, animationDelay: `${idx * 100 + 50}ms` }}
                />
                <span className="bar-value">${p.price_output_per_1m.toFixed(2)}</span>
              </div>
            </div>
          </div>
        ))}
      </div>

      <div className="bar-legend">
        <div className="legend-item">
          <div className="legend-swatch" style={{ background: 'var(--primary)' }} />
          <span>Input Price</span>
        </div>
        <div className="legend-item">
          <div className="legend-swatch" style={{ background: 'var(--accent)' }} />
          <span>Output Price</span>
        </div>
      </div>

      {tooltip && (
        <div 
          className="bar-chart-tooltip" 
          style={{ 
            left: tooltip.x, 
            top: tooltip.y, 
            transform: 'translate(-50%, -100%)' 
          }}
        >
          <div className="tooltip-title">{tooltip.name}</div>
          <div className="tooltip-item">
            <span>Input:</span>
            <strong>${tooltip.input.toFixed(2)}</strong>
          </div>
          <div className="tooltip-item">
            <span>Output:</span>
            <strong>${tooltip.output.toFixed(2)}</strong>
          </div>
        </div>
      )}
    </div>
  );
};

const DonutChart = ({ data }) => {
  const totalInput = useMemo(() => data.reduce((acc, p) => acc + p.price_input_per_1m, 0), [data]);
  
  const segments = useMemo(() => {
    let currentOffset = 0;
    const colors = ['#6366F1', '#10B981', '#8B5CF6', '#F59E0B', '#3B82F6', '#EC4899', '#06B6D4', '#84CC16'];
    
    return data.map((p, i) => {
      const percentage = (p.price_input_per_1m / totalInput) * 100;
      const strokeDasharray = `${percentage} ${100 - percentage}`;
      const strokeDashoffset = -currentOffset;
      currentOffset += percentage;
      return {
        ...p,
        percentage,
        strokeDasharray,
        strokeDashoffset,
        color: colors[i % colors.length]
      };
    });
  }, [data, totalInput]);

  return (
    <div className="panel-card">
      <div className="panel-header">
        <h3 className="panel-title">Input Share</h3>
        <p className="panel-subtitle">By provider cost</p>
      </div>

      <div className="donut-container">
        <svg viewBox="0 0 42 42" className="donut-svg" width="160" height="160">
          <circle className="donut-ring donut-bg" cx="21" cy="21" r="15.915" />
          {segments.map((seg, i) => (
            <circle
              key={seg.name}
              className="donut-ring donut-segment"
              cx="21"
              cy="21"
              r="15.915"
              stroke={seg.color}
              strokeDasharray={seg.strokeDasharray}
              strokeDashoffset={seg.strokeDashoffset}
              style={{ transitionDelay: `${i * 100}ms` }}
            />
          ))}
        </svg>
        <div className="donut-center">
          <span className="donut-center-val">{data.length}</span>
          <span className="donut-center-label">Models</span>
        </div>
      </div>

      <div className="donut-legend">
        {segments.slice(0, 5).map(seg => (
          <div key={seg.name} className="donut-legend-item">
            <div className="donut-legend-left">
              <div className="legend-swatch" style={{ background: seg.color }} />
              <span>{seg.name}</span>
            </div>
            <span style={{ color: 'var(--muted)' }}>{seg.percentage.toFixed(0)}%</span>
          </div>
        ))}
      </div>
    </div>
  );
};

const ProviderCard = ({ provider, isCheapest, maxInput, maxOutput, delay }) => {
  const [logoError, setLogoError] = useState(false);

  return (
    <div className="provider-card" style={{ animationDelay: `${delay}ms` }}>
      <div className="provider-inner">
        {isCheapest && <div className="ribbon">CHEAPEST</div>}
        
        <div className="provider-top">
          {(!provider.logo_url || logoError) ? (
            <div 
              className="logo-placeholder" 
              style={{ background: `linear-gradient(135deg, ${stringToColor(provider.name)}, #1F2937)` }}
            >
              {getInitials(provider.name)}
            </div>
          ) : (
            <img 
              src={provider.logo_url} 
              alt={provider.name} 
              className="provider-logo" 
              onError={() => setLogoError(true)}
            />
          )}
          <div className="provider-identity">
            <h4 className="provider-name">{provider.name}</h4>
            <div className="provider-desc">{provider.description}</div>
          </div>
        </div>

        <div className="price-row">
          <div className="price-pill in">
            <span className="price-pill-label">IN</span>
            <span className="price-pill-val">${provider.price_input_per_1m.toFixed(2)}</span>
          </div>
          <div className="price-pill out">
            <span className="price-pill-label">OUT</span>
            <span className="price-pill-val">${provider.price_output_per_1m.toFixed(2)}</span>
          </div>
        </div>

        <div className="mini-bar-stack">
          <div className="mini-bar-track">
            <div 
              className="mini-bar-fill" 
              style={{ 
                width: `${(provider.price_input_per_1m / maxInput) * 100}%`, 
                background: 'var(--primary)',
                animation: 'growX 0.8s ease-out backwards'
              }} 
            />
          </div>
          <div className="mini-bar-track">
            <div 
              className="mini-bar-fill" 
              style={{ 
                width: `${(provider.price_output_per_1m / maxOutput) * 100}%`, 
                background: 'var(--accent)',
                animation: 'growX 0.8s ease-out 0.1s backwards'
              }} 
            />
          </div>
        </div>
      </div>
    </div>
  );
};

const SkeletonCard = () => (
  <div className="provider-card">
    <div className="provider-inner">
      <div className="provider-top">
        <div className="skeleton-logo skeleton" />
        <div style={{ flex: 1 }}>
          <div className="skeleton-line title skeleton" />
          <div className="skeleton-line desc skeleton" />
          <div className="skeleton-line desc skeleton" style={{ width: '70%' }} />
        </div>
      </div>
      <div className="price-row" style={{ marginTop: '12px' }}>
        <div className="skeleton-pill skeleton" style={{ flex: 1 }} />
        <div className="skeleton-pill skeleton" style={{ flex: 1 }} />
      </div>
    </div>
  </div>
);

/**
 * SCREENS
 */

const LoginScreen = ({ onLoginSuccess }) => {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');
  const [shake, setShake] = useState(false);

  const handleSubmit = async (e) => {
    e.preventDefault();
    setIsLoading(true);
    setError('');
    setShake(false);

    try {
      const res = await fetch(`${API_BASE}/login`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, password })
      });
      const data = await res.json();

      if (res.ok) {
        localStorage.setItem('token', data.token);
        onLoginSuccess(data.user);
      } else {
        throw new Error(data.message || 'Invalid credentials');
      }
    } catch (err) {
      setError(err.message);
      setShake(true);
      setTimeout(() => setShake(false), 400);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="login-screen">
      <div className="aurora-container">
        <div className="aurora-blob aurora-1" />
        <div className="aurora-blob aurora-2" />
        <div className="aurora-blob aurora-3" />
      </div>

      <div className={`login-card ${shake ? 'shake' : ''}`}>
        <div className="login-header">
          <div className="login-logo">
            <Icon name="logo" size={48} />
          </div>
          <h1 className="login-title">Sakha AI Hub</h1>
          <p className="login-subtitle">Intelligence, orchestrated.</p>
        </div>

        <form className="login-form" onSubmit={handleSubmit} onChange={() => setError('')}>
          <div className="field-group">
            <input 
              type="email" 
              className="floating-input" 
              placeholder=" " 
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />
            <label className="floating-label">Email Address</label>
          </div>
          
          <div className="field-group">
            <input 
              type="password" 
              className="floating-input" 
              placeholder=" " 
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
            <label className="floating-label">Password</label>
          </div>

          {error && (
            <div className="error-banner">
              <Icon name="alert" size={16} />
              {error}
            </div>
          )}

          <button type="submit" className="submit-btn" disabled={isLoading}>
            {isLoading ? <Icon name="spinner" size={20} /> : 'Sign In'}
          </button>
        </form>

        <div className="hint-chip">
          <div className="hint-pill">
            <span>Demo: user@example.com / password123</span>
          </div>
        </div>
      </div>
    </div>
  );
};

const Dashboard = ({ user, onLogout }) => {
  const [providers, setProviders] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [search, setSearch] = useState('');
  const [sortBy, setSortBy] = useState('name');

  const fetchProviders = useCallback(async () => {
    setLoading(true);
    setError(null);
    const token = localStorage.getItem('token');
    try {
      const res = await fetch(`${API_BASE}/providers?token=${token}`);
      if (!res.ok) throw new Error('Failed to fetch providers');
      const data = await res.json();
      setProviders(data);
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchProviders();
  }, [fetchProviders]);

  const filteredAndSorted = useMemo(() => {
    let result = providers.filter(p => 
      p.name.toLowerCase().includes(search.toLowerCase()) || 
      p.description.toLowerCase().includes(search.toLowerCase())
    );

    result.sort((a, b) => {
      if (sortBy === 'name') return a.name.localeCompare(b.name);
      if (sortBy === 'price_in') return a.price_input_per_1m - b.price_input_per_1m;
      if (sortBy === 'price_out') return a.price_output_per_1m - b.price_output_per_1m;
      return 0;
    });

    return result;
  }, [providers, search, sortBy]);

  const kpis = useMemo(() => {
    if (!providers.length) return null;
    const cheapestIn = [...providers].sort((a, b) => a.price_input_per_1m - b.price_input_per_1m)[0];
    const cheapestOut = [...providers].sort((a, b) => a.price_output_per_1m - b.price_output_per_1m)[0];
    const avg = providers.reduce((acc, p) => acc + p.price_input_per_1m + p.price_output_per_1m, 0) / (providers.length * 2);
    
    return {
      total: providers.length,
      cheapestIn: { val: cheapestIn.price_input_per_1m, name: cheapestIn.name },
      cheapestOut: { val: cheapestOut.price_output_per_1m, name: cheapestOut.name },
      avgPrice: avg
    };
  }, [providers]);

  const cheapestCombinedId = useMemo(() => {
    if (!providers.length) return null;
    return providers.reduce((prev, curr) => 
      (prev.price_input_per_1m + prev.price_output_per_1m) < (curr.price_input_per_1m + curr.price_output_per_1m) ? prev : curr
    ).name;
  }, [providers]);

  const maxInput = useMemo(() => Math.max(...providers.map(p => p.price_input_per_1m), 1), [providers]);
  const maxOutput = useMemo(() => Math.max(...providers.map(p => p.price_output_per_1m), 1), [providers]);

  return (
    <div className="app-container">
      <div className="ambient-bg" />
      
      <header className="header">
        <div className="header-left">
          <div className="logo-wrap">
            <Icon name="logo" size={28} />
            <span>Sakha Hub</span>
          </div>
        </div>

        <div className="search-wrap">
          <Icon name="search" size={18} className="search-icon" />
          <input 
            type="text" 
            placeholder="Search providers or models..." 
            className="search-input"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>

        <div className="header-right">
          <div className="user-pill">
            <div className="avatar">{getInitials(user?.name)}</div>
            <span className="user-name">{user?.name}</span>
          </div>
          <button className="logout-btn" onClick={onLogout} aria-label="Logout">
            <Icon name="logout" size={20} />
          </button>
        </div>
      </header>

      <main className="main-content">
        {/* KPI ROW */}
        <section className="kpi-row" aria-label="Key Performance Indicators">
          <StatCard 
            title="Total Providers" 
            value={kpis?.total || 0} 
            icon="layers" 
            colorClass="primary" 
            delay={0}
          />
          <StatCard 
            title="Cheapest Input" 
            value={kpis?.cheapestIn.val || 0} 
            caption={kpis?.cheapestIn.name}
            icon="dollar" 
            colorClass="accent" 
            isCurrency 
            delay={100}
          />
          <StatCard 
            title="Cheapest Output" 
            value={kpis?.cheapestOut.val || 0} 
            caption={kpis?.cheapestOut.name}
            icon="chart" 
            colorClass="purple" 
            isCurrency 
            delay={200}
          />
          <StatCard 
            title="Avg. Cost / 1M" 
            value={kpis?.avgPrice || 0} 
            icon="gear" 
            colorClass="orange" 
            isCurrency 
            delay={300}
          />
        </section>

        {/* CHARTS SECTION */}
        {providers.length > 0 && (
          <section className="charts-grid" aria-label="Visual Analytics">
            <PriceBarChart data={providers} />
            <DonutChart data={providers} />
          </section>
        )}

        {/* PROVIDER GRID */}
        <section aria-label="Providers Grid">
          <div className="grid-section-header">
            <div className="grid-title-area">
              <h2 className="panel-title">Available Models</h2>
              <span className="badge">{filteredAndSorted.length}</span>
            </div>
            <select 
              className="sort-select" 
              value={sortBy} 
              onChange={(e) => setSortBy(e.target.value)}
            >
              <option value="name">Sort by: Name A-Z</option>
              <option value="price_in">Sort by: Input Price</option>
              <option value="price_out">Sort by: Output Price</option>
            </select>
          </div>

          <div className="providers-grid">
            {loading ? (
              Array(8).fill(0).map((_, i) => <SkeletonCard key={i} />)
            ) : error ? (
              <div className="error-state">
                <p>{error}</p>
                <button className="retry-btn" onClick={fetchProviders}>Retry Loading</button>
              </div>
            ) : filteredAndSorted.length === 0 ? (
              <div className="empty-state">
                <Icon name="empty" size={64} />
                <h3>No providers found</h3>
                <p>Try adjusting your search terms</p>
                <button className="clear-search-btn" onClick={() => setSearch('')}>Clear search</button>
              </div>
            ) : (
              filteredAndSorted.map((p, i) => (
                <ProviderCard 
                  key={p.name} 
                  provider={p} 
                  isCheapest={p.name === cheapestCombinedId}
                  maxInput={maxInput}
                  maxOutput={maxOutput}
                  delay={i * 60}
                />
              ))
            )}
          </div>
        </section>
      </main>
    </div>
  );
};

export default function App() {
  const [user, setUser] = useState(null);
  const [checkingAuth, setCheckingAuth] = useState(true);

  useEffect(() => {
    const restoreSession = async () => {
      const token = localStorage.getItem('token');
      if (!token) {
        setCheckingAuth(false);
        return;
      }

      try {
        const res = await fetch(`${API_BASE}/me?token=${token}`);
        if (res.ok) {
          const data = await res.json();
          setUser(data);
        } else {
          localStorage.removeItem('token');
        }
      } catch (err) {
        console.error('Session restore failed', err);
      } finally {
        setCheckingAuth(false);
      }
    };

    restoreSession();
  }, []);

  const handleLogout = async () => {
    try {
      await fetch(`${API_BASE}/logout`, { method: 'POST' });
    } catch (e) {
      console.error('Logout failed', e);
    }
    localStorage.removeItem('token');
    setUser(null);
  };

  if (checkingAuth) {
    return (
      <div className="login-screen">
        <Icon name="spinner" size={48} className="spin-icon" style={{ color: 'var(--primary)' }} />
      </div>
    );
  }

  return (
    <div className="App">
      {!user ? (
        <LoginScreen onLoginSuccess={setUser} />
      ) : (
        <Dashboard user={user} onLogout={handleLogout} />
      )}
    </div>
  );
}
