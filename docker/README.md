# Prax ORM Docker Setup

This directory contains Docker configurations for all supported databases used in development and testing.

## Quick Start

```bash
# Start all databases
docker compose up -d postgres mysql mssql mongodb

# Start a specific database
docker compose up -d postgres

# Stop all services
docker compose down

# Stop and remove volumes (clean slate)
docker compose down -v
```

## Supported Databases

### PostgreSQL

- **Port**: 5432 (host network)
- **User**: `prax`
- **Password**: `prax_test_password`
- **Database**: `prax_test`

```bash
# Start PostgreSQL
docker compose up -d postgres

# Connect
psql -h localhost -U prax -d prax_test
# Password: prax_test_password

# Run demo
cargo run --example postgres_demo
```

### MySQL

- **Port**: 3307 (host network)
- **User**: `prax`
- **Password**: `prax_test_password`
- **Database**: `prax_test`

```bash
# Start MySQL
docker compose up -d mysql

# Connect
mysql -h localhost -P 3307 -u prax -pprax_test_password prax_test

# Run demo
cargo run --example mysql_demo
```

### Microsoft SQL Server

- **Port**: 1433 (host network)
- **User**: `sa`
- **Password**: `Prax_Test_Password123!`
- **Database**: `prax_test`

```bash
# Start SQL Server
docker compose up -d mssql

# Wait for startup (about 30 seconds), then create database
docker exec prax-mssql /opt/mssql-tools/bin/sqlcmd \
  -S 127.0.0.1 -U sa -P 'Prax_Test_Password123!' -C \
  -Q "CREATE DATABASE prax_test"

# Create tables
docker exec prax-mssql /opt/mssql-tools/bin/sqlcmd \
  -S 127.0.0.1 -U sa -P 'Prax_Test_Password123!' -C -d prax_test \
  -i /docker-entrypoint-initdb.d/init.sql

# Run demo
cargo run --example mssql_demo
```

### MongoDB

- **Port**: 27017 (host network)
- **User**: `prax`
- **Password**: `prax_test_password`
- **Database**: `prax_test`

```bash
# Start MongoDB
docker compose up -d mongodb

# Connect
mongosh "mongodb://prax:prax_test_password@localhost:27017/prax_test?authSource=admin"

# Run demo
cargo run --example mongodb_demo
```

## Connection Strings

Use these connection strings in your applications:

```rust
// PostgreSQL
const PG_URL: &str = "postgresql://prax:prax_test_password@localhost:5432/prax_test";

// MySQL
const MYSQL_URL: &str = "mysql://prax:prax_test_password@localhost:3307/prax_test";

// SQL Server
const MSSQL_URL: &str = "server=localhost,1433;database=prax_test;user=sa;password=Prax_Test_Password123!;trust_server_certificate=true";

// MongoDB
const MONGO_URL: &str = "mongodb://prax:prax_test_password@localhost:27017/prax_test?authSource=admin";
```

## Running Examples

Each database has a corresponding demo example:

```bash
# PostgreSQL demo
cargo run --example postgres_demo

# MySQL demo
cargo run --example mysql_demo

# SQL Server demo
cargo run --example mssql_demo

# MongoDB demo
cargo run --example mongodb_demo
```

## Host Network Mode

This configuration uses `network_mode: host` due to Docker networking limitations on some systems. This means:

- Containers share the host's network namespace
- Port mappings in `docker-compose.yml` are for documentation only
- Services bind to `localhost` on their native ports (except MySQL which uses 3307)

## Volumes

Each database stores data in named volumes:

- `prax-postgres-data`
- `prax-mysql-data`
- `prax-mssql-data`
- `prax-mongodb-data`

To completely reset a database:

```bash
# Stop and remove the specific container and volume
docker rm -f prax-postgres
docker volume rm prax-postgres-data

# Restart
docker compose up -d postgres
```

## Health Checks

All services include health checks. Check status with:

```bash
docker compose ps
```

Expected output when healthy:

```
NAME            IMAGE                                        SERVICE    STATUS
prax-postgres   postgres:16-alpine                           postgres   Up (healthy)
prax-mysql      mysql:8.0                                    mysql      Up (healthy)
prax-mssql      mcr.microsoft.com/mssql/server:2022-latest   mssql      Up (healthy)
prax-mongodb    mongo:7.0                                    mongodb    Up (healthy)
```

## Initialization Scripts

Each database has initialization scripts in this directory:

- `postgres/init.sql` - PostgreSQL schema and seed data
- `mysql/init.sql` - MySQL schema and seed data
- `mssql/init.sql` - SQL Server schema (manual execution required)
- `mongodb/init.js` - MongoDB initialization script

## Troubleshooting

### SQL Server takes long to start

SQL Server requires more time to initialize. Wait about 30 seconds after starting:

```bash
# Check logs
docker logs prax-mssql

# Wait for "Recovery is complete" message
docker logs -f prax-mssql | grep -m1 "Recovery is complete"
```

### Can't connect to a database

1. Check if the container is running: `docker compose ps`
2. Check logs: `docker logs prax-<database>`
3. Ensure no port conflicts on your host

### Reset everything

```bash
docker compose down -v
docker compose up -d postgres mysql mssql mongodb
```
