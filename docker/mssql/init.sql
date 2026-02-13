-- =============================================================================
-- Prax ORM - SQL Server Test Database Initialization
-- =============================================================================

-- Create the test database if it doesn't exist
IF NOT EXISTS (SELECT * FROM sys.databases WHERE name = 'prax_test')
BEGIN
    CREATE DATABASE prax_test;
END
GO

USE prax_test;
GO

-- Create a non-SA user for testing (optional)
IF NOT EXISTS (SELECT * FROM sys.server_principals WHERE name = 'prax')
BEGIN
    CREATE LOGIN prax WITH PASSWORD = 'prax_test_password';
END
GO

IF NOT EXISTS (SELECT * FROM sys.database_principals WHERE name = 'prax')
BEGIN
    CREATE USER prax FOR LOGIN prax;
    ALTER ROLE db_owner ADD MEMBER prax;
END
GO

-- =============================================================================
-- Example Tables (will be created by migrations, but here for reference)
-- =============================================================================

-- Users table
IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'users')
BEGIN
    CREATE TABLE users (
        id INT IDENTITY(1,1) PRIMARY KEY,
        email NVARCHAR(255) NOT NULL UNIQUE,
        name NVARCHAR(100),
        role NVARCHAR(50) DEFAULT 'User',
        active BIT NOT NULL DEFAULT 1,
        created_at DATETIME2 NOT NULL DEFAULT GETUTCDATE(),
        updated_at DATETIME2 NOT NULL DEFAULT GETUTCDATE()
    );

    CREATE INDEX IX_users_email ON users(email);
END
GO

-- Posts table
IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'posts')
BEGIN
    CREATE TABLE posts (
        id INT IDENTITY(1,1) PRIMARY KEY,
        title NVARCHAR(255) NOT NULL,
        content NVARCHAR(MAX),
        status NVARCHAR(50) DEFAULT 'Draft',
        published BIT NOT NULL DEFAULT 0,
        views INT NOT NULL DEFAULT 0,
        created_at DATETIME2 NOT NULL DEFAULT GETUTCDATE(),
        updated_at DATETIME2 NOT NULL DEFAULT GETUTCDATE(),
        author_id INT NOT NULL,
        FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE CASCADE
    );

    CREATE INDEX IX_posts_author_id ON posts(author_id);
    CREATE INDEX IX_posts_status ON posts(status);
END
GO

-- Comments table
IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'comments')
BEGIN
    CREATE TABLE comments (
        id INT IDENTITY(1,1) PRIMARY KEY,
        content NVARCHAR(MAX) NOT NULL,
        created_at DATETIME2 NOT NULL DEFAULT GETUTCDATE(),
        updated_at DATETIME2 NOT NULL DEFAULT GETUTCDATE(),
        author_id INT,
        post_id INT NOT NULL,
        FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE SET NULL,
        FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
    );

    CREATE INDEX IX_comments_post_id ON comments(post_id);
    CREATE INDEX IX_comments_author_id ON comments(author_id);
END
GO

PRINT 'Prax test database initialized successfully!';
GO
