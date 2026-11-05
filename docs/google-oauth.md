# Your own Google app key for Gmail

Builds without bundled OAuth keys keep Gmail sign-in disabled (see
[Signing in to Gmail and Outlook](install.md#signing-in-to-gmail-and-outlook)).
You can enable it with your own Google Cloud app registration: a free
*Desktop app* OAuth client that only you, and up to 100 test users you
name, can use. This guide creates one and configures Evanya to use it.

You need a Google account and about fifteen minutes. Google renames its
console pages from time to time; if a name differs, look for the closest
match.

## What Evanya asks Google for

| Feature | Scope | API to enable |
|---|---|---|
| Mail (required) | `https://mail.google.com/`, `openid`, `email`, `profile` | Gmail API |
| Google Calendar (optional) | `https://www.googleapis.com/auth/calendar` | Google Calendar API and CalDAV API |
| Google contacts (optional) | `https://www.googleapis.com/auth/contacts.readonly` | People API |

Calendar and contacts are only requested when you connect them for an
account, so a mail-only setup needs just the Gmail API.

Sign-in uses the authorization-code flow with PKCE. Evanya receives
Google's answer on a temporary local address (`http://127.0.0.1:<port>`),
which Google allows for Desktop app clients without registering it.

## Part 1: create the app key in Google Cloud

### Step 1: create a project

1. Open the [Google Cloud console](https://console.cloud.google.com/) and
   sign in.
2. Open the project picker at the top of the page and choose **New project**.
3. Enter a name, for example `Evanya`, and choose **Create**.
4. Make sure the new project is selected in the project picker.

### Step 2: enable the APIs

1. Go to **APIs & Services → Library**.
2. Search for **Gmail API**, open it and choose **Enable**.
3. Optional, for Google Calendar: enable **Google Calendar API** and
   **CalDAV API** the same way.
4. Optional, for Google contacts: enable **People API**.

### Step 3: set up the consent screen

Google calls this part the *Google Auth Platform*. The first time you open
it, it walks you through these settings.

1. Go to **APIs & Services → OAuth consent screen** (or **Google Auth
   Platform → Overview**) and choose **Get started**.
2. **App information:** enter an app name, for example `Evanya (personal)`,
   and your email address as the user support email.
3. **Audience:** choose **External**, unless you have a Google Workspace
   organization and only its users will sign in; then choose **Internal**
   and skip step 5.
4. **Contact information:** enter your email address, accept the policy and
   choose **Create**.
5. Go to **Audience → Test users**, choose **Add users** and add the Gmail
   address of every account you will connect, including your own. While the
   app is in testing, no one else can sign in.
6. Optional but recommended: go to **Data access → Add or remove scopes**,
   add the scopes from the table above that you will use, and choose
   **Save**. The consent screen then lists them up front.

### Step 4: create the Desktop app client

1. Go to **APIs & Services → Credentials** (or **Google Auth Platform →
   Clients**).
2. Choose **Create credentials → OAuth client ID** (or **Create client**).
3. As **Application type**, choose **Desktop app**. Other types, such as Web
   application, fail with a `redirect_uri_mismatch` error.
4. Enter a name, for example `Evanya desktop`, and choose **Create**.
5. Copy the **Client ID** (it ends in `.apps.googleusercontent.com`) and the
   **Client secret** (it starts with `GOCSPX-`). You can also choose
   **Download JSON** to keep them; you can look them up again later on the
   client's page.

For a Desktop app, Google does not treat the client secret as a secret: it
cannot be kept hidden in an installed program. Still, don't publish it.

### Step 5: decide on testing or production

A new app starts in **Testing** status. This works right away, but Google
lets sign-ins with Gmail access expire after **7 days**, after which
Evanya asks you to reconnect the account.

To avoid the weekly sign-in, go to **Audience** and choose **Publish app**.
Without Google's verification this is still fine for personal use:

- Google shows a "Google hasn't verified this app" warning when you sign
  in. Choose **Advanced → Go to Evanya (unsafe)** to continue; it is
  your own app.
- At most 100 users can sign in.

Verification itself (including the security assessment Gmail access
requires) is only needed if you want to offer the app to other people.

## Part 2: configure Evanya

You can enter the key in the app, which is the usual way, or provide it
through environment variables.

### In the app, on first start

1. On the welcome screen, choose **Sign-in settings** (the cog).
2. Choose **Google**.
3. Paste the client ID into **Client ID** and the client secret into
   **Client secret (if provided)**.
4. Choose **Save settings**. The status line changes to "Using your saved
   Google app keys", and the Gmail button on the welcome screen becomes
   available.

### In the app, later

1. Open **Settings → Accounts**.
2. In the **OAuth app credentials** card, choose **Configure**.
3. Paste the client ID into **Gmail desktop client ID** and the client
   secret into **Gmail client secret (optional)**.
4. Choose **Save Gmail** (**Save Gmail settings** on a phone).

Saved keys are shared between the welcome screen and Settings, and take
precedence over keys built into the app. Clear the client ID and save to go
back to the built-in keys, if your build has them.

### Connect your Gmail account

1. Choose **Add Gmail** in **Settings → Accounts**, or the Gmail button on
   the welcome screen.
2. Your browser opens Google's sign-in page. Choose the account you added
   as a test user.
3. If Google shows the unverified-app warning, choose **Advanced → Go to
   … (unsafe)**.
4. Allow access. The browser tab then says you can close it, and Flectar
   Mail starts synchronizing the account.

For calendar and contacts, find the account in **Settings → Accounts** and
choose **Connect Google Calendar**, or **Google contacts** followed by
**Connect**. Your browser opens again and Google asks for the extra
permission.

### With environment variables

Instead of saving the key in the app you can provide it when you start
Evanya. Keys saved in the app and keys built into the app take
precedence over these.

```sh
export FLECTAR_MAIL_GOOGLE_CLIENT_ID="1234567890-abc.apps.googleusercontent.com"
export FLECTAR_MAIL_GOOGLE_CLIENT_SECRET="GOCSPX-..."
evanya
```

### Built into your own build

When you build from source, the key can be compiled in, so every
installation of that build has Gmail sign-in enabled. Set these variables
in the environment of the build, or put them in a `.env` file in the root
of the repository:

```sh
FLECTAR_GOOGLE_DESKTOP_CLIENT_ID="1234567890-abc.apps.googleusercontent.com"
FLECTAR_GOOGLE_DESKTOP_CLIENT_SECRET="GOCSPX-..."
```

Then build as usual, for example with `make release`. Anyone who has the
binary can extract the key from it, so only do this for builds you keep to
yourself.

## Troubleshooting

| Problem | Cause and fix |
|---|---|
| The Gmail button stays disabled | No key is configured. Check the status line in **Sign-in settings**. |
| "Access blocked: … has not completed the Google verification process" | The account is not a test user. Add it under **Audience → Test users**, or publish the app (step 5). |
| `Error 400: redirect_uri_mismatch` | The client is not a Desktop app client. Create a new client with type **Desktop app** (step 4). |
| `invalid_client` or `unauthorized_client` | The client ID or secret was copied incompletely, or belongs to another project or a deleted client. Copy both again. |
| Sign-in works, but mail does not synchronize | The Gmail API is not enabled in this project (step 2). |
| Connecting Google Calendar or contacts fails | Enable the Google Calendar API and CalDAV API, or the People API, in the same project. |
| You have to sign in again every week | The app is in Testing status. Publish it (step 5). |
| The browser does not return to Evanya | A firewall or proxy blocks connections to `127.0.0.1`. Allow local connections for the browser, then try again. |
