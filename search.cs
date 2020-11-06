using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Data;
using System.Drawing;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Windows.Forms;
using System.Configuration;
using MySql.Data.MySqlClient;
using System.Net;
using System.Net.Sockets;
using System.Data.SqlClient;
using System.IO;
using System.Net.NetworkInformation;

namespace pokerpractice
{
    public partial class search : Form
    {
        
        public search(string userName)
        {
            InitializeComponent();
            user = userName;
        }
        int myRank = 0, oppRank = 0;
        MySqlCommand command;
        string ip = "",user="",opponent = "",opponentIP = "";
        string connStr = ConfigurationManager.ConnectionStrings["connstr"].ConnectionString;
        private void search_Load(object sender, EventArgs e)
        {
            Ping ping = new Ping();
            PingReply pingReply = ping.Send("1.1.1.1");
            if (pingReply.Status == IPStatus.Success)
            {
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                        SELECT c.UserRank
                        FROM ranks AS c
                        WHERE c.UserID = @user
                        ORDER BY c.UserRank";
                        command.Parameters.AddWithValue("@user", user);
                        conn.Open();
                        using (var reader = command.ExecuteReader())
                        {
                            while (reader.Read())
                            {
                                myRank = reader.GetInt32(0);
                            }
                        }
                        conn.Close();
                    }
                }
                userLabel.Text = "" + user+": "+myRank;
                double rankPic = Math.Floor((double)myRank / 100);
                if (rankPic < 10)
                {
                    rankPictureBox.Image = thepokerladder.Properties.Resources.bronzechip;
                }
                else
                {
                    switch (rankPic)
                    {
                        case 10:
                            rankPictureBox.Image = thepokerladder.Properties.Resources.silverchip;
                            break;
                        case 11:
                            rankPictureBox.Image = thepokerladder.Properties.Resources.goldchip;
                            break;
                        case 12:
                            rankPictureBox.Image = thepokerladder.Properties.Resources.diamondchip;
                            break;
                        default:
                            rankPictureBox.Image = thepokerladder.Properties.Resources.masterchip;
                            break;
                    }
                }
            }
            else
            {
                button2_Click(this, new EventArgs());
            }
        }

        private void button1_Click(object sender, EventArgs e)
        {
            searchButton.Enabled = false;
            try
            {
                IPAddress[] localIP = Dns.GetHostAddresses(Dns.GetHostName());
                foreach (IPAddress adress in localIP)
                {
                    if (adress.AddressFamily == AddressFamily.InterNetwork)
                    {
                        ip = adress.ToString();
                    }
                }
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                            UPDATE ranks
                            SET ranks.UserActive = @active
                            WHERE ranks.UserID = @userID";
                        command.Parameters.AddWithValue("@active", 1);
                        command.Parameters.AddWithValue("@userID", user);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                            UPDATE ranks
                            SET ranks.IPAddress = @ip
                            WHERE ranks.UserID = @userID";
                        command.Parameters.AddWithValue("@ip", ip);
                        command.Parameters.AddWithValue("@userID", user);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                            SELECT c.UserRank
                            FROM ranks AS c
                            WHERE c.UserID = @user
                            ORDER BY c.UserRank;";
                        command.Parameters.AddWithValue("@user", user);
                        conn.Open();
                        using (var reader = command.ExecuteReader())
                        {
                            while (reader.Read())
                            {
                                myRank = reader.GetInt32(0);
                            }
                        }
                        conn.Close();
                    }
                }
                matchmake();
            }
            catch (Exception err)
            {
                MessageBox.Show(err.Message);
                button2_Click(this, new EventArgs());
            }
        }

        List<idRank> idRanks = new List<idRank>();

        public class idRank
        {
            public string id { get; set; }
            public int rank { get; set; }
        }

        public async void matchmake()
        {
            command = new MySqlCommand("");
            using (var conn = new MySqlConnection(connStr))
            {
                using (command = conn.CreateCommand())
                {
                    command.CommandText = @"
                        SELECT c.HostUser
                        FROM match AS c
                        WHERE c.ClientUser = @user";
                    command.Parameters.AddWithValue("@user", user);
                    conn.Open();
                    using (var reader = command.ExecuteReader())
                    {
                        while (reader.Read())
                        {
                            opponent = reader.GetString(0);
                        }
                    }
                    conn.Close();
                }
            }
            command = new MySqlCommand("");
            using (var conn = new MySqlConnection(connStr))
            {
                using (command = conn.CreateCommand())
                {
                    command.CommandText = @"
                        SELECT c.IPAddress, c.UserRank
                        FROM ranks AS c
                        WHERE c.UserID = @user";
                    command.Parameters.AddWithValue("@user", opponent);
                    conn.Open();
                    using (var reader = command.ExecuteReader())
                    {
                        while (reader.Read())
                        {
                            opponentIP = reader.GetString(0);
                            oppRank = reader.GetInt32(1);
                        }
                    }
                    conn.Close();
                }
            }
            if (opponent != "")
            {
                //launch poker with the ips of both users
                this.Hide();
                poker frm = new poker(user, opponent, opponentIP, false, myRank, oppRank);
                frm.ShowDialog();
                this.Close();
            }
            else
            {
                var IR = new idRank();
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                        SELECT c.UserID, c.UserRank
                        FROM ranks AS c
                        WHERE c.UserActive = @active AND c.UserID != @user
                        ORDER BY c.UserRank";
                        command.Parameters.AddWithValue("@active", 1);
                        command.Parameters.AddWithValue("@user", user);
                        conn.Open();
                        using (var reader = command.ExecuteReader())
                        {
                            while (reader.Read())
                            {
                                IR.id = reader.GetString(0);
                                IR.rank = reader.GetInt32(1);
                                idRanks.Add(IR);
                            }
                        }
                        conn.Close();
                    }
                }
                if (idRanks.Count!=0) {
                    string bestMatch = "";
                    int bestDis = Math.Abs(myRank - idRanks[0].rank), currentDis = 0;
                    bestMatch = idRanks[0].id;
                    for (int i = 1; i < idRanks.Count; i++)
                    {
                        currentDis = Math.Abs(myRank - idRanks[i].rank);
                        if (currentDis < bestDis)
                        {
                            bestDis = currentDis;
                            bestMatch = idRanks[i].id;
                        }
                    }
                    command = new MySqlCommand("");
                    using (var conn = new MySqlConnection(connStr))
                    {
                        using (command = conn.CreateCommand())
                        {
                            command.CommandText = @"
                            SELECT c.IPAddress, c.UserRank
                            FROM ranks AS c
                            WHERE c.UserID = @user";
                            command.Parameters.AddWithValue("@user", bestMatch);
                            conn.Open();
                            using (var reader = command.ExecuteReader())
                            {
                                while (reader.Read())
                                {
                                    opponentIP = reader.GetString(0);
                                    oppRank = reader.GetInt32(1);
                                }
                            }
                            conn.Close();
                        }
                    }
                    Ping ping = new Ping();
                    PingReply pingReply = ping.Send(opponentIP);
                    if (pingReply.Status == IPStatus.Success) {
                        command = new MySqlCommand("");
                        using (var conn = new MySqlConnection(connStr))
                        {
                            using (command = conn.CreateCommand())
                            {
                                command.CommandText = @"
                                INSERT INTO match (match.HostUser, match.ClientUser)
                                VALUES (@host, @client);";
                                command.Parameters.AddWithValue("@host", user);
                                command.Parameters.AddWithValue("@client", bestMatch);
                                conn.Open();
                                command.ExecuteNonQuery();
                                conn.Close();
                            }
                        }
                        this.Hide();
                        poker frm = new poker(user, bestMatch, opponentIP, true, myRank, oppRank);
                        frm.ShowDialog();
                        this.Close();
                    }
                    else
                    {
                        await Task.Delay(1000);
                        matchmake();
                    }
                }
                else
                {
                    await Task.Delay(1000);
                    matchmake();
                }
            }
        }

        private void search_FormClosing(object sender, FormClosingEventArgs e)
        {
            try
            {
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                        UPDATE ranks
                        SET ranks.UserActive = @active
                        WHERE ranks.UserID = @userID";
                        command.Parameters.AddWithValue("@active", 0);
                        command.Parameters.AddWithValue("@userID", user);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
            }
            catch (Exception ex)
            {
                MessageBox.Show("Logout failed: "+ex);
            }
        }

        private void button2_Click(object sender, EventArgs e)
        {
            this.Hide();
            login logform = new login();
            logform.ShowDialog();
            this.Close();
        }
    }
}
