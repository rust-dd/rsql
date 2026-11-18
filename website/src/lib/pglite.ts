import { PGlite } from "@electric-sql/pglite";

let db: PGlite | null = null;
let initPromise: Promise<PGlite> | null = null;

const SEED_SQL = `
  CREATE TABLE employees (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    department TEXT NOT NULL,
    salary NUMERIC(10,2),
    hire_date DATE,
    is_active BOOLEAN DEFAULT true
  );

  CREATE TABLE departments (
    id SERIAL PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    budget NUMERIC(12,2),
    location TEXT
  );

  CREATE TABLE projects (
    id SERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    department_id INTEGER REFERENCES departments(id),
    start_date DATE,
    status TEXT DEFAULT 'active'
  );

  INSERT INTO departments (name, budget, location) VALUES
    ('Engineering', 2500000, 'San Francisco'),
    ('Marketing', 800000, 'New York'),
    ('Sales', 1200000, 'Chicago'),
    ('Design', 600000, 'Remote'),
    ('Data Science', 1800000, 'San Francisco');

  INSERT INTO employees (name, department, salary, hire_date, is_active) VALUES
    ('Alice Chen', 'Engineering', 145000, '2021-03-15', true),
    ('Bob Smith', 'Marketing', 92000, '2022-07-01', true),
    ('Carol Williams', 'Engineering', 138000, '2020-11-20', true),
    ('David Brown', 'Sales', 88000, '2023-01-10', true),
    ('Eve Davis', 'Design', 105000, '2021-08-22', true),
    ('Frank Miller', 'Engineering', 155000, '2019-05-14', true),
    ('Grace Lee', 'Data Science', 142000, '2022-02-28', true),
    ('Henry Wilson', 'Sales', 95000, '2023-06-15', false),
    ('Iris Taylor', 'Marketing', 87000, '2022-09-01', true),
    ('Jack Anderson', 'Data Science', 150000, '2021-04-10', true),
    ('Karen Thomas', 'Engineering', 160000, '2018-12-01', true),
    ('Leo Martinez', 'Design', 98000, '2023-03-20', true);

  INSERT INTO projects (title, department_id, start_date, status) VALUES
    ('API Redesign', 1, '2024-01-15', 'active'),
    ('Brand Refresh', 2, '2024-03-01', 'active'),
    ('ML Pipeline', 5, '2023-11-01', 'active'),
    ('Mobile App v2', 1, '2024-02-01', 'planning'),
    ('Sales Dashboard', 3, '2023-09-15', 'completed'),
    ('Design System', 4, '2024-01-01', 'active');
`;

export async function getDB(): Promise<PGlite> {
  if (db) return db;
  if (initPromise) return initPromise;

  initPromise = (async () => {
    const candidate = new PGlite();
    try {
      await candidate.exec(SEED_SQL);
      db = candidate;
      return candidate;
    } catch (error) {
      initPromise = null;
      await candidate.close().catch(() => undefined);
      throw error;
    }
  })();

  return initPromise;
}

export async function resetDB(): Promise<PGlite> {
  if (db) {
    await db.close();
    db = null;
    initPromise = null;
  }
  return getDB();
}
