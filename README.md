# team_bot
Internal team bot for notification about some events

# Start
Bot using sqlx, so you need execute following command:
- sqlx db create
- sqlx migrate run

The bot currently supports three types of tasks:
* Notification Task \
A notification task can be created manually or imported from the Exchange calendar.
* Task for Checking Failed Build Configurations in TeamCity
* Task for Reviewing Gerrit Changes Based on Specific Conditions