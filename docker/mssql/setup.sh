#!/bin/bash
# =============================================================================
# Prax ORM - SQL Server Setup Script
# =============================================================================
# This script waits for SQL Server to start, then runs the init.sql

# Wait for SQL Server to start
echo "Waiting for SQL Server to start..."
for i in {1..60}; do
    /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -C -Q "SELECT 1" &>/dev/null
    if [ $? -eq 0 ]; then
        echo "SQL Server is ready!"
        break
    fi
    echo "Waiting... ($i/60)"
    sleep 2
done

# Run the initialization script
echo "Running initialization script..."
/opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -C -i /docker-entrypoint-initdb.d/init.sql

echo "Setup complete!"
