\# UI ↔ Backend Contract



\## Purpose



This document defines the contract between the frontend (UI/UX) and backend (GPU tuning and service layer).



The goal is to:



\- document IPC methods

\- document payloads

\- track requested cross-team changes

\- avoid breaking integrations



\---



\# Current IPC Methods



To be documented as the frontend/backend contract stabilizes.



\---



\# Change Requests



(No active requests)



\---



\# Rules



Backend may:

\- add new optional fields

\- add new IPC methods



Backend must not:

\- rename payload fields without updating this document

\- remove fields without migration notes



Frontend must:

\- tolerate missing optional fields

\- avoid relying on display strings for logic

\- use structured payload fields whenever possible



Frontend must not:

\- infer safety state from logs

\- infer profile state from text messages

