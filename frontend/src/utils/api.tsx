import axios from "axios";
import { BACKEND_URL } from "../hooks/auth";

export const apiGet = async (endpoint: string) => {
  const token = localStorage.getItem("token");
  const headers = {};
  if (!!token) {
    headers["authorization"] = "Bearer " + token;
  }
  return await axios.get(BACKEND_URL + endpoint, { headers });
};
