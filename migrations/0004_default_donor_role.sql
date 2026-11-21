INSERT INTO user_roles(user_id,role_id)
SELECT u.id,r.id
FROM users u
JOIN roles r ON r.name='donor'
WHERE NOT EXISTS (
    SELECT 1 FROM user_roles ur WHERE ur.user_id=u.id
)
ON CONFLICT DO NOTHING;
