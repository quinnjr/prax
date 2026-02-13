-- =============================================================================
-- Prax ORM - PostgreSQL Initialization Script
-- =============================================================================
-- This script runs when the PostgreSQL container is first created.
-- It sets up the test database with proper permissions and extensions.

-- Enable useful extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "vector";

-- Create additional test databases for isolation
CREATE DATABASE prax_test_migrations;
CREATE DATABASE prax_test_integration;

-- Grant permissions
GRANT ALL PRIVILEGES ON DATABASE prax_test TO prax;
GRANT ALL PRIVILEGES ON DATABASE prax_test_migrations TO prax;
GRANT ALL PRIVILEGES ON DATABASE prax_test_integration TO prax;

-- Connect to main test database and set up schema
\c prax_test

-- Create a sample schema for testing (will be overwritten by migrations)
CREATE TABLE IF NOT EXISTS _prax_migrations (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE,
    applied_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- =============================================================================
-- Benchmark Tables
-- =============================================================================

-- Users table for benchmarking and demos
CREATE TABLE IF NOT EXISTS users (
    id BIGSERIAL PRIMARY KEY,
    name VARCHAR(255),
    email VARCHAR(255) NOT NULL UNIQUE,
    age INTEGER NOT NULL DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    role VARCHAR(50) NOT NULL DEFAULT 'User',
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    score INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Posts table for relation benchmarking
CREATE TABLE IF NOT EXISTS posts (
    id BIGSERIAL PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    content TEXT,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    published BOOLEAN NOT NULL DEFAULT FALSE,
    view_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Create indexes for benchmark queries
CREATE INDEX IF NOT EXISTS idx_users_status ON users(status);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at);
CREATE INDEX IF NOT EXISTS idx_posts_user_id ON posts(user_id);
CREATE INDEX IF NOT EXISTS idx_posts_published ON posts(published);

-- Seed some initial data for benchmarks
INSERT INTO users (name, email, age, active, status, role, verified, score)
SELECT
    'User ' || i,
    'user' || i || '@example.com',
    20 + (i % 50),
    i % 10 != 0,  -- active: 90% true
    CASE WHEN i % 10 = 0 THEN 'inactive' ELSE 'active' END,
    CASE WHEN i % 100 = 0 THEN 'Admin' WHEN i % 20 = 0 THEN 'Moderator' ELSE 'User' END,
    i % 3 = 0,
    (i * 17) % 1000
FROM generate_series(1, 1000) AS i
ON CONFLICT (email) DO NOTHING;

-- Seed posts
INSERT INTO posts (title, content, user_id, published, view_count)
SELECT
    'Post ' || i || ' by User ' || ((i % 1000) + 1),
    'Content for post ' || i,
    ((i % 1000) + 1),
    i % 5 != 0,
    (i * 13) % 10000
FROM generate_series(1, 5000) AS i;

-- =============================================================================
-- pgvector Test Tables
-- =============================================================================

-- Dense vector embeddings (3-dimensional for simple testing)
CREATE TABLE IF NOT EXISTS embeddings (
    id SERIAL PRIMARY KEY,
    content TEXT NOT NULL,
    embedding vector(3) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Document table with vector + full-text search
CREATE TABLE IF NOT EXISTS documents (
    id SERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    embedding vector(4),
    sparse_embedding sparsevec(4),
    search_vector tsvector,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Binary feature vectors
CREATE TABLE IF NOT EXISTS binary_features (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    features bit(8) NOT NULL
);

-- Full-text search index for hybrid search tests
CREATE INDEX IF NOT EXISTS idx_documents_search ON documents USING gin(search_vector);

-- Seed embeddings
INSERT INTO embeddings (content, embedding) VALUES
    ('cat', '[1.0, 0.0, 0.0]'),
    ('dog', '[0.9, 0.1, 0.0]'),
    ('fish', '[0.0, 1.0, 0.0]'),
    ('bird', '[0.0, 0.0, 1.0]'),
    ('hamster', '[0.8, 0.2, 0.1]');

-- Seed documents with vectors and tsvectors
INSERT INTO documents (title, body, embedding, search_vector) VALUES
    ('Introduction to AI', 'Artificial intelligence is transforming industries', '[0.1, 0.2, 0.3, 0.4]', to_tsvector('english', 'Artificial intelligence is transforming industries')),
    ('Machine Learning Basics', 'ML models learn from data patterns', '[0.5, 0.6, 0.7, 0.8]', to_tsvector('english', 'ML models learn from data patterns')),
    ('Neural Networks', 'Deep learning uses layers of neurons', '[0.9, 0.1, 0.2, 0.3]', to_tsvector('english', 'Deep learning uses layers of neurons')),
    ('Natural Language Processing', 'NLP enables computers to understand text', '[0.3, 0.4, 0.5, 0.6]', to_tsvector('english', 'NLP enables computers to understand text')),
    ('Computer Vision', 'Image recognition and object detection', '[0.7, 0.8, 0.9, 0.1]', to_tsvector('english', 'Image recognition and object detection'));

-- Seed binary features
INSERT INTO binary_features (name, features) VALUES
    ('feature_a', B'10101010'),
    ('feature_b', B'01010101'),
    ('feature_c', B'11110000'),
    ('feature_d', B'00001111');

-- Vector indexes for search tests
CREATE INDEX IF NOT EXISTS idx_embeddings_hnsw ON embeddings USING hnsw (embedding vector_l2_ops);
CREATE INDEX IF NOT EXISTS idx_documents_hnsw ON documents USING hnsw (embedding vector_cosine_ops);

-- Log initialization
DO $$
BEGIN
    RAISE NOTICE 'Prax PostgreSQL test database initialized successfully';
    RAISE NOTICE 'Seeded % users and % posts', (SELECT COUNT(*) FROM users), (SELECT COUNT(*) FROM posts);
    RAISE NOTICE 'Seeded % embeddings, % documents, % binary features', (SELECT COUNT(*) FROM embeddings), (SELECT COUNT(*) FROM documents), (SELECT COUNT(*) FROM binary_features);
END $$;

