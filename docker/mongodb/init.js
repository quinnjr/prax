// =============================================================================
// Prax ORM - MongoDB Test Database Initialization
// =============================================================================

// Switch to the test database
db = db.getSiblingDB('prax_test');

// Create collections with schema validation
db.createCollection('users', {
    validator: {
        $jsonSchema: {
            bsonType: 'object',
            required: ['email', 'active', 'created_at', 'updated_at'],
            properties: {
                email: {
                    bsonType: 'string',
                    description: 'User email address - required'
                },
                name: {
                    bsonType: ['string', 'null'],
                    description: 'User display name'
                },
                role: {
                    enum: ['User', 'Admin', 'Moderator'],
                    description: 'User role'
                },
                active: {
                    bsonType: 'bool',
                    description: 'Whether user is active'
                },
                created_at: {
                    bsonType: 'date',
                    description: 'Creation timestamp'
                },
                updated_at: {
                    bsonType: 'date',
                    description: 'Last update timestamp'
                }
            }
        }
    }
});

// Create unique index on email
db.users.createIndex({ email: 1 }, { unique: true });

db.createCollection('posts', {
    validator: {
        $jsonSchema: {
            bsonType: 'object',
            required: ['title', 'published', 'views', 'author_id', 'created_at', 'updated_at'],
            properties: {
                title: {
                    bsonType: 'string',
                    description: 'Post title - required'
                },
                content: {
                    bsonType: ['string', 'null'],
                    description: 'Post content'
                },
                status: {
                    enum: ['Draft', 'Published', 'Archived'],
                    description: 'Post status'
                },
                published: {
                    bsonType: 'bool',
                    description: 'Whether post is published'
                },
                views: {
                    bsonType: 'int',
                    description: 'View count'
                },
                author_id: {
                    bsonType: 'objectId',
                    description: 'Reference to author user'
                },
                created_at: {
                    bsonType: 'date',
                    description: 'Creation timestamp'
                },
                updated_at: {
                    bsonType: 'date',
                    description: 'Last update timestamp'
                }
            }
        }
    }
});

// Create indexes
db.posts.createIndex({ author_id: 1 });
db.posts.createIndex({ status: 1 });
db.posts.createIndex({ published: 1 });

db.createCollection('comments', {
    validator: {
        $jsonSchema: {
            bsonType: 'object',
            required: ['content', 'post_id', 'created_at', 'updated_at'],
            properties: {
                content: {
                    bsonType: 'string',
                    description: 'Comment content - required'
                },
                author_id: {
                    bsonType: ['objectId', 'null'],
                    description: 'Reference to author user'
                },
                post_id: {
                    bsonType: 'objectId',
                    description: 'Reference to post - required'
                },
                created_at: {
                    bsonType: 'date',
                    description: 'Creation timestamp'
                },
                updated_at: {
                    bsonType: 'date',
                    description: 'Last update timestamp'
                }
            }
        }
    }
});

// Create indexes
db.comments.createIndex({ post_id: 1 });
db.comments.createIndex({ author_id: 1 });

// Grant permissions to prax user
db.createUser({
    user: 'prax_user',
    pwd: 'prax_test_password',
    roles: [
        { role: 'readWrite', db: 'prax_test' }
    ]
});

print('Prax MongoDB test database initialized successfully!');
