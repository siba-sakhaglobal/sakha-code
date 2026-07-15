const express = require('express');
const Database = require('better-sqlite3');
const crypto = require('crypto');
const cors = require('cors');
const path = require('path');

const app = express();
app.use(express.json());
app.use(cors());

const db = new Database('data.sqlite');

// Initialize DB
db.exec(`
  CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT UNIQUE,
    password_hash TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );
  CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(user_id) REFERENCES users(id)
  );
`);

// Migration/Reset for providers table to ensure UNIQUE(name) constraint
db.exec(`DROP TABLE IF EXISTS providers;
  CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE,
    logo_url TEXT,
    price_input_per_1m REAL,
    price_output_per_1m REAL,
    description TEXT
  );
`);

// Seed demo user
const salt = 'fixed_salt_for_demo';
const hashPassword = (pass) => crypto.scryptSync(pass, salt, 64).toString('hex');

const stmt = db.prepare('SELECT count(*) as count FROM users WHERE email = ?');
if (stmt.get('demo@example.com').count === 0) {
    db.prepare('INSERT INTO users (email, password_hash) VALUES (?, ?)').run('demo@example.com', hashPassword('password123'));
}

// Seed Providers
const providers = [
    ['OpenAI GPT-4o-mini', 'https://cdn.simpleicons.org/openai', 0.15, 0.60, 'Efficient model by OpenAI'],
    ['Anthropic Claude 3.5 Sonnet', 'https://cdn.simpleicons.org/anthropic', 3.00, 15.00, 'Powerful reasoning model'],
    ['Google Gemini 2.0 Flash', 'https://cdn.simpleicons.org/googlegemini', 0.10, 0.40, 'Fast multimodal model'],
    ['Mistral Small', 'https://cdn.simpleicons.org/mistralai', 0.20, 0.60, 'Cost-effective European model'],
    ['DeepSeek Chat', 'https://cdn.simpleicons.org/deepseek', 0.14, 0.28, 'High performance coding focus'],
    ['Groq Llama 3.3 70B', 'https://cdn.simpleicons.org/groq', 0.59, 0.79, 'Inference speed optimized'],
    ['xAI Grok', 'https://cdn.simpleicons.org/xai', 2.00, 6.00, 'Real-time knowledge focus'],
    ['OpenRouter Auto', 'https://cdn.simpleicons.org/openrouter', 0.20, 0.50, 'Intelligent model routing']
];

const insertProvider = db.prepare(`
    INSERT OR REPLACE INTO providers (name, logo_url, price_input_per_1m, price_output_per_1m, description) 
    VALUES (?, ?, ?, ?, ?)
`);
providers.forEach(p => insertProvider.run(...p));

// Endpoints
app.post('/api/login', (req, res) => {
    const { email, password } = req.body;
    const user = db.prepare('SELECT * FROM users WHERE email = ?').get(email);
    if (!user || user.password_hash !== hashPassword(password)) {
        return res.status(401).json({ error: 'Invalid credentials' });
    }
    const token = crypto.randomBytes(32).toString('hex');
    db.prepare('INSERT INTO sessions (token, user_id) VALUES (?, ?)').run(token, user.id);
    res.json({ token, user: { id: user.id, email: user.email } });
});

app.post('/api/logout', (req, res) => {
    db.prepare('DELETE FROM sessions WHERE token = ?').run(req.body.token);
    res.json({ success: true });
});

app.get('/api/me', (req, res) => {
    const token = req.query.token;
    const session = db.prepare('SELECT u.* FROM users u JOIN sessions s ON u.id = s.user_id WHERE s.token = ?').get(token);
    if (!session) return res.status(401).json({ error: 'Unauthorized' });
    res.json(session);
});

app.get('/api/providers', (req, res) => {
    const token = req.query.token;
    if (!db.prepare('SELECT 1 FROM sessions WHERE token = ?').get(token)) {
        return res.status(401).json({ error: 'Unauthorized' });
    }
    res.json(db.prepare('SELECT * FROM providers').all());
});

app.listen(5051, () => console.log('Backend running on http://localhost:5051'));
