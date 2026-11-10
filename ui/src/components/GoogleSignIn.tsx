import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useId, useState } from "react";
import { connectGoogle, signInGoogle } from "../lib/runtime";
import { Failure } from "./ui";

// Signing in to Google, shared by setup and Settings. Picket's own client is one click;
// a person's own Google Cloud client stays available as the advanced path.
//
// Buttons stay enabled while Google is pending: each attempt listens on its own port, so
// a new attempt simply replaces one whose browser tab was closed, instead of leaving the
// person waiting out the 15-minute limit.
export function GoogleSignIn({ picketClient }: { picketClient: boolean }) {
  const client = useQueryClient();
  const id = useId();
  const [own, setOwn] = useState(!picketClient);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const signIn = useMutation({
    mutationFn: () => signInGoogle(),
    onSuccess: () => client.invalidateQueries(),
  });
  const connect = useMutation({
    mutationFn: () => connectGoogle(clientId.trim(), clientSecret),
    onSuccess: () => {
      setClientSecret("");
      return client.invalidateQueries();
    },
  });
  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    connect.mutate();
  }
  return (
    <div className="stack google-sign-in">
      {picketClient && (
        <div>
          <button type="button" className="button primary" onClick={() => signIn.mutate()}>
            {signIn.isPending ? "Open Google sign-in again" : "Sign in with Google"}
          </button>
          <p className="caption google-sign-in-note" aria-live="polite">
            {signIn.isPending
              ? "Waiting for you to finish in the browser. If you closed it, open it again."
              : "Your browser opens Google’s sign-in. Google hasn’t reviewed Picket yet, so it shows a warning: choose Advanced, then Go to Picket."}
          </p>
          {signIn.isError && <Failure error={signIn.error} />}
        </div>
      )}
      {picketClient && !own && (
        <button type="button" className="link-button" onClick={() => setOwn(true)}>
          Use your own Google Cloud client instead
        </button>
      )}
      {own && (
        <form className="setting-form google-sign-in-form" onSubmit={onSubmit}>
          <label htmlFor={`${id}-client-id`}>Google Desktop OAuth client ID</label>
          <input
            id={`${id}-client-id`}
            autoComplete="off"
            value={clientId}
            onChange={(event) => setClientId(event.target.value)}
            required
          />
          <label htmlFor={`${id}-client-secret`}>Client secret, if supplied</label>
          <input
            id={`${id}-client-secret`}
            type="password"
            autoComplete="off"
            value={clientSecret}
            onChange={(event) => setClientSecret(event.target.value)}
          />
          <div>
            <button type="submit" className={picketClient ? "button" : "button primary"}>
              {connect.isPending ? "Open Google sign-in again" : "Sign in with this client"}
            </button>
          </div>
          {connect.isError && <Failure error={connect.error} />}
        </form>
      )}
    </div>
  );
}
